plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

configurations.configureEach {
    resolutionStrategy.force(
        "androidx.lifecycle:lifecycle-common:2.8.7",
        "androidx.lifecycle:lifecycle-common-jvm:2.8.7",
        "androidx.lifecycle:lifecycle-livedata:2.8.7",
        "androidx.lifecycle:lifecycle-livedata-core:2.8.7",
        "androidx.lifecycle:lifecycle-livedata-core-ktx:2.8.7",
        "androidx.lifecycle:lifecycle-process:2.8.7",
        "androidx.lifecycle:lifecycle-runtime:2.8.7",
        "androidx.lifecycle:lifecycle-runtime-android:2.8.7",
        "androidx.lifecycle:lifecycle-runtime-compose:2.8.7",
        "androidx.lifecycle:lifecycle-runtime-compose-android:2.8.7",
        "androidx.lifecycle:lifecycle-runtime-ktx:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-android:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-compose-android:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-ktx:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-savedstate:2.8.7",
        "androidx.lifecycle:lifecycle-viewmodel-savedstate-android:2.8.7",
        "androidx.savedstate:savedstate:1.2.1",
        "androidx.savedstate:savedstate-android:1.2.1",
        "androidx.savedstate:savedstate-ktx:1.2.1",
    )
}

val composeCompilerPlugin = "androidx.compose.compiler:compiler:1.5.15"
configurations.matching { it.name.startsWith("kotlinCompilerPluginClasspath") }.configureEach {
    project.dependencies.add(name, composeCompilerPlugin)
}

android {
    compileSdk = 36
    namespace = "com.kexin94yyds.iterate"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.kexin94yyds.iterate"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "false"
            isDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        buildConfig = true
        compose = true
    }
    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.15"
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.runtime:runtime")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("io.noties.markwon:core:4.6.2")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20260522")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}
