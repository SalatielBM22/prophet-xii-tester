package org.prophetxii;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.Locale;
import org.json.JSONObject;

public final class MainActivity extends Activity {
    private static final String ASSET_HOST = "appassets.androidplatform.net";
    private static final String START_URL = "https://" + ASSET_HOST + "/index.html";

    private WebView webView;
    private LinearLayout errorBanner;
    private TextView errorText;
    private volatile SynthService boundService;
    private boolean serviceBound = false;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final Runnable healthPollTask = new Runnable() {
        @Override
        public void run() {
            updateBannerFromDiagnostics();
            mainHandler.postDelayed(this, 1000L);
        }
    };

    private final ServiceConnection connection = new ServiceConnection() {
        @Override
        public void onServiceConnected(ComponentName name, IBinder service) {
            if (service instanceof SynthService.LocalBinder) {
                boundService = ((SynthService.LocalBinder) service).getService();
            }
            updateBannerFromDiagnostics();
        }

        @Override
        public void onServiceDisconnected(ComponentName name) {
            boundService = null;
        }
    };

    static String accessToken(Context context) {
        return SynthService.accessToken(context);
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(new String[]{"android.permission.POST_NOTIFICATIONS"}, 1);
        }

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.parseColor("#11100E"));

        webView = new WebView(this);
        webView.setBackgroundColor(Color.parseColor("#11100E"));
        configureWebView(webView);
        root.addView(webView, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));

        // Slim native rescue strip shown ONLY when AAudio is in FAILED state or WebView fails to load assets.
        errorBanner = new LinearLayout(this);
        errorBanner.setOrientation(LinearLayout.HORIZONTAL);
        errorBanner.setGravity(Gravity.CENTER_VERTICAL);
        errorBanner.setPadding(24, 14, 24, 14);
        errorBanner.setBackgroundColor(Color.parseColor("#E63A1713"));
        errorBanner.setVisibility(View.GONE);

        errorText = new TextView(this);
        errorText.setTextColor(Color.parseColor("#FFE4D6"));
        errorText.setTextSize(12.5f);
        LinearLayout.LayoutParams textLp = new LinearLayout.LayoutParams(
                0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f);
        errorBanner.addView(errorText, textLp);

        Button diagBtn = new Button(this);
        diagBtn.setText("Diagnostics");
        diagBtn.setOnClickListener(v -> showDiagnosticsDialog());
        errorBanner.addView(diagBtn);

        Button retryBtn = new Button(this);
        retryBtn.setText("Retry Audio");
        retryBtn.setOnClickListener(v -> triggerAudioRetry());
        errorBanner.addView(retryBtn);

        FrameLayout.LayoutParams bannerLp = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP);
        root.addView(errorBanner, bannerLp);

        setContentView(root);

        Intent svcIntent = new Intent(this, SynthService.class);
        try {
            startForegroundService(svcIntent);
        } catch (RuntimeException ignored) {
            startService(svcIntent);
        }
        serviceBound = bindService(svcIntent, connection, Context.BIND_AUTO_CREATE);

        webView.loadUrl(START_URL);
        mainHandler.postDelayed(healthPollTask, 750L);
    }

    private void configureWebView(WebView view) {
        WebSettings ws = view.getSettings();
        ws.setJavaScriptEnabled(true);
        ws.setDomStorageEnabled(true);
        ws.setAllowFileAccess(false);
        ws.setAllowContentAccess(false);
        ws.setMediaPlaybackRequiresUserGesture(false);
        ws.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);

        view.addJavascriptInterface(new ProphetAndroidBridge(), "ProphetAndroidBridge");
        view.setWebViewClient(new WebViewClient() {
            @Override
            public WebResourceResponse shouldInterceptRequest(
                    WebView wv, WebResourceRequest request) {
                Uri uri = request.getUrl();
                if (uri == null || !"https".equalsIgnoreCase(uri.getScheme())
                        || !ASSET_HOST.equalsIgnoreCase(uri.getHost())) {
                    return null;
                }
                return servePackagedAsset(uri.getPath());
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView wv, WebResourceRequest request) {
                Uri uri = request.getUrl();
                return uri == null || !ASSET_HOST.equalsIgnoreCase(uri.getHost());
            }

            @Override
            public void onReceivedError(
                    WebView wv, WebResourceRequest request, WebResourceError error) {
                if (request != null && request.isForMainFrame()) {
                    showBannerMessage("WebView asset error (" + error.getErrorCode()
                            + "): " + error.getDescription());
                }
            }
        });
    }

    private WebResourceResponse servePackagedAsset(String rawPath) {
        String rel = (rawPath == null || "/".equals(rawPath) || rawPath.isEmpty())
                ? "index.html"
                : rawPath.startsWith("/") ? rawPath.substring(1) : rawPath;
        if (rel.contains("..") || rel.contains("\\")) {
            return plainTextResponse(400, "Bad Request", "Invalid asset path");
        }
        try {
            InputStream stream = getAssets().open(rel);
            return new WebResourceResponse(mimeTypeForAsset(rel), "UTF-8", stream);
        } catch (IOException missing) {
            // SPA fallback only for extension-less routes; report genuine 404 for missing scripts/css.
            if (!rel.contains(".")) {
                try {
                    InputStream index = getAssets().open("index.html");
                    return new WebResourceResponse("text/html", "UTF-8", index);
                } catch (IOException ignored) {
                    // Fall through to helpful missing-asset error.
                }
            }
            return plainTextResponse(404, "Not Found", "Missing packaged UI asset: " + rel);
        }
    }

    private static WebResourceResponse plainTextResponse(int code, String reason, String body) {
        ByteArrayInputStream data = new ByteArrayInputStream(body.getBytes(StandardCharsets.UTF_8));
        return new WebResourceResponse(
                "text/plain",
                "UTF-8",
                code,
                reason,
                java.util.Collections.emptyMap(),
                data);
    }

    private static String mimeTypeForAsset(String path) {
        String lower = path.toLowerCase(Locale.ROOT);
        if (lower.endsWith(".html")) return "text/html";
        if (lower.endsWith(".js") || lower.endsWith(".mjs")) return "application/javascript";
        if (lower.endsWith(".css")) return "text/css";
        if (lower.endsWith(".json")) return "application/json";
        if (lower.endsWith(".svg")) return "image/svg+xml";
        if (lower.endsWith(".png")) return "image/png";
        if (lower.endsWith(".wasm")) return "application/wasm";
        if (lower.endsWith(".woff2")) return "font/woff2";
        return "application/octet-stream";
    }

    private void triggerAudioRetry() {
        SynthService svc = boundService;
        if (svc != null) {
            svc.retryAudioAsync();
        } else {
            Intent retry = new Intent(this, SynthService.class).setAction("RETRY_AUDIO");
            startService(retry);
        }
        updateBannerFromDiagnostics();
    }

    private void updateBannerFromDiagnostics() {
        String diagJson = Native.diagnosticsJson();
        if (diagJson == null || diagJson.isEmpty()) {
            return;
        }
        try {
            JSONObject obj = new JSONObject(diagJson);
            String lifecycle = obj.optString("lifecycle", "STOPPED");
            if ("FAILED".equals(lifecycle)) {
                int code = obj.optInt("lastErrorCode", -1);
                String text = obj.optString("lastErrorText", "Unknown AAudio error");
                showBannerMessage("AAudio failed (" + code + ": " + text + "). UI & MIDI offline ready.");
            } else {
                errorBanner.setVisibility(View.GONE);
            }
        } catch (Exception ignored) {
            // Keep existing visibility safely.
        }
    }

    private void showBannerMessage(String msg) {
        mainHandler.post(() -> {
            errorText.setText(msg);
            errorBanner.setVisibility(View.VISIBLE);
        });
    }

    private void showDiagnosticsDialog() {
        String raw = Native.diagnosticsJson();
        String pretty = raw;
        try {
            pretty = new JSONObject(raw).toString(2);
        } catch (Exception ignored) {
        }
        SynthService svc = boundService;
        String bridgeInfo = (svc != null) ? svc.describeDevBridgeStatus() : "Service binding pending";
        new AlertDialog.Builder(this)
                .setTitle("PROPHET-XII Android Audio & USB Diagnostics")
                .setMessage(bridgeInfo + "\n\n" + pretty)
                .setPositiveButton("Retry Audio", (d, w) -> triggerAudioRetry())
                .setNegativeButton("Close", null)
                .show();
    }

    @Override
    protected void onDestroy() {
        mainHandler.removeCallbacks(healthPollTask);
        if (serviceBound) {
            try {
                unbindService(connection);
            } catch (IllegalArgumentException ignored) {
            }
            serviceBound = false;
        }
        if (webView != null) {
            webView.removeJavascriptInterface("ProphetAndroidBridge");
            webView.destroy();
            webView = null;
        }
        super.onDestroy();
    }

    final class ProphetAndroidBridge {
        @JavascriptInterface
        public String dispatch(String cmdJson) {
            SynthService svc = boundService;
            if (svc != null) {
                return svc.dispatchFromUi(cmdJson);
            }
            return Native.dispatchJson(cmdJson);
        }

        @JavascriptInterface
        public String pollSnapshot() {
            return Native.pollSnapshotJson();
        }

        @JavascriptInterface
        public String diagnostics() {
            return Native.diagnosticsJson();
        }
    }
}
