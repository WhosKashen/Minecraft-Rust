// gamehost: starts a desktop-style OpenJDK (built for Android/bionic) inside the :game process and
// runs Minecraft's main class. It also exposes the game surface and an input-handler hook to a native
// graphics/windowing bridge (see docs/NATIVE_BRIDGE.md). This file does NOT implement SDL or OpenGL/Vulkan.
#include <android/log.h>
#include <android/native_window_jni.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <jni.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define TAG "gamehost"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, TAG, __VA_ARGS__)

typedef jint (*CreateJavaVM_t)(JavaVM **, void **, void *);

static ANativeWindow *g_window = NULL;
static int g_w = 0, g_h = 0;
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
typedef void (*input_handler_t)(const char *event);
static input_handler_t g_input = NULL;
static unsigned long g_dropped = 0;

// ---- API for the native bridge (resolve with dlsym(RTLD_DEFAULT, ...)) ----------------------
__attribute__((visibility("default"))) void gamehost_set_input_handler(input_handler_t h) { g_input = h; }
__attribute__((visibility("default"))) ANativeWindow *gamehost_get_window(int *w, int *h) {
    pthread_mutex_lock(&g_lock);
    ANativeWindow *win = g_window;
    if (w) *w = g_w;
    if (h) *h = g_h;
    pthread_mutex_unlock(&g_lock);
    return win;
}

JNIEXPORT void JNICALL Java_dev_rustcraft_launcher_GameNative_setSurface(JNIEnv *env, jclass c, jobject surface) {
    (void)c;
    pthread_mutex_lock(&g_lock);
    if (g_window) { ANativeWindow_release(g_window); g_window = NULL; }
    if (surface) g_window = ANativeWindow_fromSurface(env, surface);
    pthread_mutex_unlock(&g_lock);
}

JNIEXPORT void JNICALL Java_dev_rustcraft_launcher_GameNative_surfaceChanged(JNIEnv *env, jclass c, jint w, jint h) {
    (void)env; (void)c;
    pthread_mutex_lock(&g_lock);
    g_w = w; g_h = h;
    pthread_mutex_unlock(&g_lock);
}

JNIEXPORT jboolean JNICALL Java_dev_rustcraft_launcher_GameNative_inject(JNIEnv *env, jclass c, jstring ev) {
    (void)c;
    input_handler_t h = g_input;
    if (!h) {
        if ((++g_dropped % 500) == 1) LOGI("input dropped: no native bridge attached (%lu so far)", g_dropped);
        return JNI_FALSE;
    }
    const char *s = (*env)->GetStringUTFChars(env, ev, NULL);
    if (s) { h(s); (*env)->ReleaseStringUTFChars(env, ev, s); }
    return JNI_TRUE;
}

static char *dup_jstr(JNIEnv *env, jstring s) {
    const char *u = (*env)->GetStringUTFChars(env, s, NULL);
    char *r = strdup(u ? u : "");
    if (u) (*env)->ReleaseStringUTFChars(env, s, u);
    return r;
}

JNIEXPORT jint JNICALL Java_dev_rustcraft_launcher_GameNative_runJvm(
    JNIEnv *env, jclass c, jstring jlibjvm, jstring jhome, jobjectArray jvmArgs, jstring jmain, jobjectArray gameArgs,
    jstring jwork, jobjectArray envKeys, jobjectArray envVals, jstring jlog) {
    (void)c;
    char *libjvm = dup_jstr(env, jlibjvm), *home = dup_jstr(env, jhome), *mainc = dup_jstr(env, jmain);
    char *work = dup_jstr(env, jwork), *logf = dup_jstr(env, jlog);
    jint rc = -1;

    // Log JVM stdout/stderr to the file the launcher shows on its Logs page.
    int fd = open(logf, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd >= 0) { dup2(fd, 1); dup2(fd, 2); close(fd); }
    setvbuf(stdout, NULL, _IOLBF, 0);

    if (chdir(work) != 0) LOGE("chdir(%s) failed", work);
    jsize ne = (*env)->GetArrayLength(env, envKeys);
    for (jsize i = 0; i < ne; i++) {
        jstring k = (jstring)(*env)->GetObjectArrayElement(env, envKeys, i), v = (jstring)(*env)->GetObjectArrayElement(env, envVals, i);
        char *ks = dup_jstr(env, k), *vs = dup_jstr(env, v);
        setenv(ks, vs, 1);
        free(ks); free(vs);
    }
    setenv("JAVA_HOME", home, 1);

    // Copy every argument out of ART's JNIEnv before the OpenJDK VM exists (two VMs share this thread).
    jsize na = (*env)->GetArrayLength(env, jvmArgs), ng = (*env)->GetArrayLength(env, gameArgs);
    char **jargs = calloc((size_t)na + 1, sizeof *jargs), **gargs = calloc((size_t)ng + 1, sizeof *gargs);
    for (jsize i = 0; i < na; i++) jargs[i] = dup_jstr(env, (jstring)(*env)->GetObjectArrayElement(env, jvmArgs, i));
    for (jsize i = 0; i < ng; i++) gargs[i] = dup_jstr(env, (jstring)(*env)->GetObjectArrayElement(env, gameArgs, i));

    // Bionic reads LD_LIBRARY_PATH only at process start, so dependent runtime libraries are preloaded by absolute path.
    const char *pre[] = {"lib/libjava.so", "lib/libverify.so", "lib/libnet.so", "lib/libnio.so", NULL};
    void *jvmh = dlopen(libjvm, RTLD_NOW | RTLD_GLOBAL);
    if (!jvmh) { LOGE("dlopen(%s) failed: %s", libjvm, dlerror()); fprintf(stderr, "Cannot load libjvm: %s\n", dlerror()); goto done; }
    for (int i = 0; pre[i]; i++) { char p[4096]; snprintf(p, sizeof p, "%s/%s", home, pre[i]); dlopen(p, RTLD_NOW | RTLD_GLOBAL); }

    CreateJavaVM_t create = (CreateJavaVM_t)dlsym(jvmh, "JNI_CreateJavaVM");
    if (!create) { LOGE("JNI_CreateJavaVM not found"); goto done; }

    JavaVMOption *opts = calloc((size_t)na + 2, sizeof *opts);
    char homeopt[4200]; snprintf(homeopt, sizeof homeopt, "-Djava.home=%s", home);
    opts[0].optionString = homeopt;
    for (jsize i = 0; i < na; i++) opts[i + 1].optionString = jargs[i];
    JavaVMInitArgs args = {.version = JNI_VERSION_1_8, .nOptions = na + 1, .options = opts, .ignoreUnrecognized = JNI_TRUE};

    JavaVM *vm = NULL; JNIEnv *jenv = NULL;
    jint cr = create(&vm, (void **)&jenv, &args);
    if (cr != JNI_OK) { LOGE("JNI_CreateJavaVM failed: %d", cr); fprintf(stderr, "JNI_CreateJavaVM failed: %d\n", cr); goto done; }

    for (char *p = mainc; *p; p++) if (*p == '.') *p = '/';
    jclass mc = (*jenv)->FindClass(jenv, mainc);
    jmethodID mm = mc ? (*jenv)->GetStaticMethodID(jenv, mc, "main", "([Ljava/lang/String;)V") : NULL;
    if (!mm) { (*jenv)->ExceptionDescribe(jenv); LOGE("main class/method not found: %s", mainc); goto done; }
    jclass sc = (*jenv)->FindClass(jenv, "java/lang/String");
    jobjectArray ga = (*jenv)->NewObjectArray(jenv, ng, sc, NULL);
    for (jsize i = 0; i < ng; i++) {
        (*jenv)->SetObjectArrayElement(jenv, ga, i, (*jenv)->NewStringUTF(jenv, gargs[i]));
    }
    (*jenv)->CallStaticVoidMethod(jenv, mc, mm, ga);
    rc = 0;
    if ((*jenv)->ExceptionCheck(jenv)) { (*jenv)->ExceptionDescribe(jenv); rc = 1; }
    (*vm)->DetachCurrentThread(vm);
    (*vm)->DestroyJavaVM(vm);   // waits for non-daemon threads (the game's render thread)
done:
    free(libjvm); free(home); free(mainc); free(work); free(logf);
    return rc;
}
