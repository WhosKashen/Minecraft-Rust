plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.rustcraft.launcher"
    compileSdk = 35
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "dev.rustcraft.launcher"
        minSdk = 26          // Vulkan 1.0 + AAudio baseline; Java 25 runtime and the game itself need far more in practice
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a") }   // only arm64 is supported (see docs/COMPATIBILITY.md)
        externalNativeBuild { cmake { cppFlags += "" } }
    }
    externalNativeBuild { cmake { path = file("src/main/cpp/CMakeLists.txt"); version = "3.22.1" } }
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")   // cargo-ndk output goes here
    packaging { jniLibs { useLegacyPackaging = true } }      // native libs must be extracted to disk
    buildTypes { release { isMinifyEnabled = false } }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    kotlinOptions { jvmTarget = "17" }
}
