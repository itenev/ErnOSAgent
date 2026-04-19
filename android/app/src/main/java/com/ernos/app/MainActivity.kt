package com.ernos.app

import android.annotation.SuppressLint
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowInsetsController
import android.webkit.*
import androidx.appcompat.app.AppCompatActivity

/**
 * Main activity — fullscreen WebView pointing to the local Ern-OS engine.
 * The engine runs as a foreground service (EngineService) so it persists
 * when the user switches apps.
 *
 * Displays a loading screen while the engine boots, polling until ready.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        // Fullscreen immersive
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.insetsController?.apply {
                hide(android.view.WindowInsets.Type.statusBars() or android.view.WindowInsets.Type.navigationBars())
                systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            }
        } else {
            @Suppress("DEPRECATION")
            window.decorView.systemUiVisibility = (
                View.SYSTEM_UI_FLAG_FULLSCREEN
                    or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    or View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
            )
        }

        // Start the engine service
        val serviceIntent = Intent(this, EngineService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(serviceIntent)
        } else {
            startService(serviceIntent)
        }

        // Configure WebView
        webView = findViewById(R.id.webView)
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            allowFileAccess = true
            mediaPlaybackRequiresUserGesture = false
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
            javaScriptCanOpenWindowsAutomatically = true
            useWideViewPort = true
            loadWithOverviewMode = true
        }

        webView.webViewClient = object : WebViewClient() {
            override fun onReceivedError(
                view: WebView?, request: WebResourceRequest?,
                error: WebResourceError?
            ) {
                // Only retry for the main frame, not sub-resources
                if (request?.isForMainFrame == true) {
                    view?.postDelayed({
                        view.loadUrl("http://127.0.0.1:3000")
                    }, 2000)
                }
            }
        }

        webView.webChromeClient = WebChromeClient()

        // Show loading screen immediately, then poll for engine readiness
        showLoadingScreen()
    }

    /** Load an inline HTML loading page that polls the engine health endpoint. */
    private fun showLoadingScreen() {
        val loadingHtml = """
            <!DOCTYPE html>
            <html>
            <head>
                <meta charset="UTF-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
                <style>
                    * { margin: 0; padding: 0; box-sizing: border-box; }
                    body {
                        background: #06060e;
                        color: #e0e0e0;
                        font-family: -apple-system, sans-serif;
                        display: flex;
                        flex-direction: column;
                        align-items: center;
                        justify-content: center;
                        min-height: 100vh;
                        padding: 24px;
                    }
                    .logo {
                        font-size: 3rem;
                        font-weight: 700;
                        background: linear-gradient(135deg, #00FF88, #3b82f6);
                        -webkit-background-clip: text;
                        -webkit-text-fill-color: transparent;
                        margin-bottom: 24px;
                    }
                    .spinner {
                        width: 40px; height: 40px;
                        border: 3px solid rgba(0,255,136,0.15);
                        border-top-color: #00FF88;
                        border-radius: 50%;
                        animation: spin 1s linear infinite;
                        margin-bottom: 20px;
                    }
                    @keyframes spin { to { transform: rotate(360deg); } }
                    .status {
                        font-size: 0.9rem;
                        color: rgba(255,255,255,0.5);
                        text-align: center;
                    }
                    .dots::after {
                        content: '';
                        animation: dots 1.5s steps(4, end) infinite;
                    }
                    @keyframes dots {
                        0% { content: ''; }
                        25% { content: '.'; }
                        50% { content: '..'; }
                        75% { content: '...'; }
                    }
                </style>
            </head>
            <body>
                <div class="logo">Ern-OS</div>
                <div class="spinner"></div>
                <div class="status" id="status">Starting engine<span class="dots"></span></div>
                <script>
                    let attempts = 0;
                    function checkEngine() {
                        attempts++;
                        const status = document.getElementById('status');
                        fetch('http://127.0.0.1:3000/api/health')
                            .then(r => {
                                if (r.ok) {
                                    status.innerHTML = 'Connected!';
                                    setTimeout(() => {
                                        window.location.href = 'http://127.0.0.1:3000';
                                    }, 300);
                                } else {
                                    scheduleRetry();
                                }
                            })
                            .catch(() => {
                                if (attempts < 10) {
                                    status.innerHTML = 'Starting engine<span class="dots"></span>';
                                } else if (attempts < 30) {
                                    status.innerHTML = 'Loading model<span class="dots"></span>';
                                } else {
                                    status.innerHTML = 'Still loading — this can take a minute on first launch<span class="dots"></span>';
                                }
                                scheduleRetry();
                            });
                    }
                    function scheduleRetry() {
                        setTimeout(checkEngine, 1000);
                    }
                    setTimeout(checkEngine, 1500);
                </script>
            </body>
            </html>
        """.trimIndent()

        webView.loadDataWithBaseURL(null, loadingHtml, "text/html", "UTF-8", null)
    }

    override fun onBackPressed() {
        if (webView.canGoBack()) {
            webView.goBack()
        } else {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    override fun onDestroy() {
        webView.destroy()
        super.onDestroy()
    }
}
