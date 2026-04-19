package com.ernos.app

/**
 * Compute mode determines where inference runs.
 * Saved in SharedPreferences and changeable from the Settings UI.
 */
enum class ComputeMode(val key: String, val label: String, val description: String) {
    LOCAL("local", "Local", "Everything runs on-device. Fully offline-capable."),
    HYBRID("hybrid", "Hybrid", "Small tasks on-device, heavy compute forwarded to host."),
    HOST("host", "Host", "All inference on host machine. Phone is a thin client.");

    companion object {
        fun fromKey(key: String): ComputeMode =
            entries.firstOrNull { it.key == key } ?: LOCAL
    }
}
