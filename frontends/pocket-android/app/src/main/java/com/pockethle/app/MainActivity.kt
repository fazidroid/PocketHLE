package com.pockethle.app

import android.app.Activity
import android.os.Bundle
import android.widget.TextView

class MainActivity : Activity() {
    init {
        // Loads libpockethle_jni.so packaged with the APK. Built by:
        //
        //   cargo ndk -t arm64-v8a -o app/src/main/jniLibs \
        //       build --release -p pocket-android-jni
        System.loadLibrary("pockethle_jni")
    }

    private external fun banner(): String

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val tv = TextView(this).apply {
            text = banner()
            textSize = 18f
            setPadding(48, 96, 48, 48)
        }
        setContentView(tv)
    }
}
