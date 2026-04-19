package com.ernos.app

import android.util.Log

/**
 * Manages connection to a host Ern-OS instance for Hybrid/Host compute modes.
 * Handles manual IP entry and QR code pairing.
 */
class HostConnector {

    companion object {
        private const val TAG = "ErnOS.Host"
    }

    data class HostInfo(
        val ip: String,
        val port: Int = 3000,
        val name: String = "Ern-OS Host"
    ) {
        val baseUrl: String get() = "http://$ip:$port"
        val providerUrl: String get() = "http://$ip:8080/v1/chat/completions"
    }

    /**
     * Parse a QR code payload into HostInfo.
     * Expected format: ernos://<ip>:<port> or just <ip>:<port>
     */
    fun parseQrCode(payload: String): HostInfo? {
        return try {
            val cleaned = payload
                .removePrefix("ernos://")
                .removePrefix("http://")
                .removePrefix("https://")
                .trim()

            val parts = cleaned.split(":")
            val ip = parts[0]
            val port = parts.getOrNull(1)?.toIntOrNull() ?: 3000

            if (ip.isEmpty()) return null

            Log.i(TAG, "Parsed host: $ip:$port")
            HostInfo(ip = ip, port = port)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse QR code: $payload", e)
            null
        }
    }

    /**
     * Test connectivity to a host instance.
     * Must be called from a background thread.
     */
    fun testConnection(host: HostInfo): Boolean {
        return try {
            val url = java.net.URL("${host.baseUrl}/api/health")
            val conn = url.openConnection() as java.net.HttpURLConnection
            conn.connectTimeout = 5000
            conn.readTimeout = 5000
            val code = conn.responseCode
            conn.disconnect()
            code == 200
        } catch (e: Exception) {
            Log.w(TAG, "Host unreachable: ${host.baseUrl} — ${e.message}")
            false
        }
    }
}
