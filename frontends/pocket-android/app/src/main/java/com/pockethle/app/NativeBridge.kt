package com.pockethle.app

/**
 * Thin Kotlin facade around the Rust JNI library `libpockethle_jni.so`.
 *
 * Every native method takes the absolute path of the library root as
 * its first argument and returns a JSON-encoded string. JSON is used
 * as the lingua franca to keep the FFI surface minimal — no shared
 * data classes between Rust and Kotlin.
 */
object NativeBridge {
    init {
        System.loadLibrary("pockethle_jni")
    }

    @JvmStatic external fun banner(): String

    /** Returns a JSON array of [GameEntry]. */
    @JvmStatic external fun listGames(libraryRoot: String): String

    /** Returns the freshly imported game as a JSON [GameEntry]. */
    @JvmStatic external fun importCab(libraryRoot: String, cabPath: String): String

    /** Returns `{"ok":true}` on success or `{"ok":false,"error":...}`. */
    @JvmStatic external fun removeGame(libraryRoot: String, id: String): String

    /** Returns the launcher config as JSON. */
    @JvmStatic external fun readConfig(libraryRoot: String): String

    /** Persists the launcher config from a JSON blob. */
    @JvmStatic external fun writeConfig(libraryRoot: String, configJson: String): String

    /** Returns the per-game settings as JSON. */
    @JvmStatic external fun readGameSettings(libraryRoot: String, id: String): String

    /** Persists the per-game settings from a JSON blob. */
    @JvmStatic external fun writeGameSettings(
        libraryRoot: String,
        id: String,
        settingsJson: String,
    ): String

    /**
     * Run the emulator for [id] and return JSON of the form
     * `{"ok":true,"summary":"...","frame":{"width":W,"height":H,"rgba_b64":"..."}}`
     * or `{"ok":false,"error":"..."}`.
     */
    @JvmStatic external fun runGame(libraryRoot: String, id: String): String
}
