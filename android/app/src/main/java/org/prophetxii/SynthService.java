package org.prophetxii;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.media.AudioAttributes;
import android.media.AudioDeviceCallback;
import android.media.AudioDeviceInfo;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.media.midi.MidiDevice;
import android.media.midi.MidiDeviceInfo;
import android.media.midi.MidiManager;
import android.media.midi.MidiOutputPort;
import android.media.midi.MidiReceiver;
import android.os.Binder;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;
import java.io.ByteArrayOutputStream;
import java.io.Closeable;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

public final class SynthService extends Service {
    final class LocalBinder extends Binder {
        SynthService getService() {
            return SynthService.this;
        }
    }

    private final IBinder binder = new LocalBinder();
    private volatile boolean serviceAlive = false;
    private volatile boolean wantAudioRunning = true;
    private volatile boolean devBridgeEnabled = false;
    private volatile String cachedToken = "";
    private volatile String currentAudioRoute = "speaker";

    private ServerSocket devServer;
    private final Set<Socket> sockets = ConcurrentHashMap.newKeySet();
    private final ExecutorService workers = Executors.newFixedThreadPool(6);
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    private final Handler mainHandler = new Handler(Looper.getMainLooper());

    private PowerManager.WakeLock wakeLock;
    private AudioManager audioManager;
    private AudioFocusRequest focusRequest;
    private AudioDeviceCallback audioDeviceCallback;

    private MidiManager midiManager;
    private MidiManager.DeviceCallback midiDeviceCallback;
    private final Map<Integer, OpenMidiDevice> openMidiDevices = new ConcurrentHashMap<>();

    private final Object streamLock = new Object();
    private int consecutiveRecoveries = 0;
    private long lastRecoveryMs = 0L;
    private long lastHealthyStartMs = 0L;

    private static final class OpenMidiDevice implements Closeable {
        final String label;
        final MidiDevice device;
        final List<MidiOutputPort> ports;

        OpenMidiDevice(String label, MidiDevice device, List<MidiOutputPort> ports) {
            this.label = label;
            this.device = device;
            this.ports = ports;
        }

        @Override
        public void close() {
            for (MidiOutputPort port : ports) {
                try {
                    port.close();
                } catch (IOException ignored) {
                }
            }
            try {
                device.close();
            } catch (IOException ignored) {
            }
        }
    }

    static String accessToken(Context context) {
        SharedPreferences prefs = context.getSharedPreferences("control", MODE_PRIVATE);
        String token = prefs.getString("token", null);
        if (token == null || token.isEmpty()) {
            byte[] raw = new byte[24];
            new SecureRandom().nextBytes(raw);
            token = android.util.Base64.encodeToString(
                    raw, android.util.Base64.NO_WRAP | android.util.Base64.URL_SAFE);
            prefs.edit().putString("token", token).apply();
        }
        return token;
    }

    @Override
    public void onCreate() {
        super.onCreate();
        cachedToken = accessToken(this);
        serviceAlive = true;

        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null) {
            nm.createNotificationChannel(new NotificationChannel(
                    "audio", "PROPHET-XII Audio", NotificationManager.IMPORTANCE_LOW));
        }
        Intent stopIntent = new Intent(this, SynthService.class).setAction("STOP");
        PendingIntent stopPending = PendingIntent.getService(
                this, 1, stopIntent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification note = new Notification.Builder(this, "audio")
                .setContentTitle("PROPHET-XII Synthesizer")
                .setContentText("Offline WebView UI active · Tap Stop to halt audio")
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setOngoing(true)
                .addAction(new Notification.Action.Builder(null, "Stop", stopPending).build())
                .build();
        startForeground(12, note);

        audioManager = getSystemService(AudioManager.class);
        initAudioFocus();
        registerAudioDeviceMonitor();
        registerUsbMidiMonitor();

        PowerManager pm = getSystemService(PowerManager.class);
        if (pm != null) {
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "PROPHET-XII:Audio");
            wakeLock.acquire(10 * 60 * 60 * 1000L);
        }

        workers.submit(this::startAudioInternal);
        scheduler.scheduleAtFixedRate(this::monitorAudioHealth, 120L, 120L, TimeUnit.MILLISECONDS);
    }

    private void initAudioFocus() {
        if (audioManager == null) {
            return;
        }
        AudioAttributes attrs = new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build();
        focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(attrs)
                .setAcceptsDelayedFocusGain(true)
                .setWillPauseWhenDucked(false)
                .setOnAudioFocusChangeListener(this::handleAudioFocusChange)
                .build();
        int res = audioManager.requestAudioFocus(focusRequest);
        if (res == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
            Native.setFocusGain("AUDIOFOCUS_GAIN", false, 1.0f);
        } else if (res == AudioManager.AUDIOFOCUS_REQUEST_DELAYED) {
            Native.setFocusGain("AUDIOFOCUS_DELAYED", true, 0.0f);
        } else {
            Native.setFocusGain("AUDIOFOCUS_REQUEST_FAILED", true, 0.0f);
        }
    }

    private void handleAudioFocusChange(int change) {
        switch (change) {
            case AudioManager.AUDIOFOCUS_GAIN:
                Native.setFocusGain("AUDIOFOCUS_GAIN", false, 1.0f);
                if (wantAudioRunning && Native.healthy() != 0) {
                    retryAudioAsync();
                }
                break;
            case AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK:
                Native.setFocusGain("AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK", false, 0.25f);
                break;
            case AudioManager.AUDIOFOCUS_LOSS_TRANSIENT:
                Native.setFocusGain("AUDIOFOCUS_LOSS_TRANSIENT", true, 0.0f);
                break;
            case AudioManager.AUDIOFOCUS_LOSS:
                // Smoothly mute cleanly rather than tearing down the activity/UI control plane.
                Native.setFocusGain("AUDIOFOCUS_LOSS", true, 0.0f);
                break;
            default:
                break;
        }
    }

    private void registerAudioDeviceMonitor() {
        if (audioManager == null) {
            return;
        }
        audioDeviceCallback = new AudioDeviceCallback() {
            @Override
            public void onAudioDevicesAdded(AudioDeviceInfo[] added) {
                refreshRouteInfo();
            }

            @Override
            public void onAudioDevicesRemoved(AudioDeviceInfo[] removed) {
                refreshRouteInfo();
            }
        };
        audioManager.registerAudioDeviceCallback(audioDeviceCallback, mainHandler);
        refreshRouteInfo();
    }

    private void registerUsbMidiMonitor() {
        if (!getPackageManager().hasSystemFeature(PackageManager.FEATURE_MIDI)) {
            return;
        }
        try {
            midiManager = getSystemService(MidiManager.class);
        } catch (RuntimeException ignored) {
            midiManager = null;
        }
        if (midiManager == null) {
            return;
        }
        midiDeviceCallback = new MidiManager.DeviceCallback() {
            @Override
            public void onDeviceAdded(MidiDeviceInfo device) {
                openMidiDeviceIfController(device);
            }

            @Override
            public void onDeviceRemoved(MidiDeviceInfo device) {
                OpenMidiDevice closed = openMidiDevices.remove(device.getId());
                if (closed != null) {
                    closed.close();
                }
                refreshRouteInfo();
            }
        };
        midiManager.registerDeviceCallback(midiDeviceCallback, mainHandler);
        for (MidiDeviceInfo info : midiManager.getDevices()) {
            openMidiDeviceIfController(info);
        }
    }

    private void openMidiDeviceIfController(MidiDeviceInfo info) {
        if (midiManager == null || info.getOutputPortCount() <= 0) {
            return;
        }
        final int devId = info.getId();
        final String product = info.getProperties().getString(MidiDeviceInfo.PROPERTY_NAME);
        final String label = (product != null && !product.isEmpty()) ? product : ("USB-MIDI-" + devId);
        midiManager.openDevice(info, device -> {
            if (device == null || !serviceAlive) {
                return;
            }
            List<MidiOutputPort> ports = new ArrayList<>();
            int count = info.getOutputPortCount();
            for (int p = 0; p < count; p++) {
                MidiOutputPort port = device.openOutputPort(p);
                if (port != null) {
                    port.connect(new MidiReceiver() {
                        @Override
                        public void onSend(byte[] msg, int offset, int cnt, long timestamp) {
                            Native.usbMidi(msg, offset, cnt);
                        }
                    });
                    ports.add(port);
                }
            }
            if (ports.isEmpty()) {
                try {
                    device.close();
                } catch (IOException ignored) {
                }
                return;
            }
            OpenMidiDevice prev = openMidiDevices.put(devId, new OpenMidiDevice(label, device, ports));
            if (prev != null) {
                prev.close();
            }
            refreshRouteInfo();
        }, mainHandler);
    }

    private void refreshRouteInfo() {
        String route = "speaker";
        if (audioManager != null) {
            AudioDeviceInfo[] outputs = audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS);
            for (AudioDeviceInfo dev : outputs) {
                int t = dev.getType();
                if (t == AudioDeviceInfo.TYPE_USB_DEVICE
                        || t == AudioDeviceInfo.TYPE_USB_HEADSET
                        || t == AudioDeviceInfo.TYPE_USB_ACCESSORY) {
                    route = "usb-audio";
                    break;
                } else if (t == AudioDeviceInfo.TYPE_WIRED_HEADPHONES
                        || t == AudioDeviceInfo.TYPE_WIRED_HEADSET) {
                    route = "wired-headset";
                } else if (t == AudioDeviceInfo.TYPE_BLUETOOTH_A2DP && "speaker".equals(route)) {
                    route = "bluetooth-a2dp";
                }
            }
        }
        currentAudioRoute = route;
        StringBuilder csv = new StringBuilder();
        for (OpenMidiDevice dev : openMidiDevices.values()) {
            if (csv.length() > 0) {
                csv.append(',');
            }
            csv.append(dev.label.replace(',', ' '));
        }
        Native.setRouteInfo(route, csv.toString());
    }

    private int[] queryAudioHints() {
        int sr = 48000;
        int burst = 192;
        if (audioManager != null) {
            try {
                String srProp = audioManager.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE);
                if (srProp != null) {
                    int parsed = Integer.parseInt(srProp.trim());
                    if (parsed >= 8000 && parsed <= 192000) {
                        sr = parsed;
                    }
                }
            } catch (NumberFormatException ignored) {
            }
            try {
                String burstProp = audioManager.getProperty(
                        AudioManager.PROPERTY_OUTPUT_FRAMES_PER_BUFFER);
                if (burstProp != null) {
                    int parsed = Integer.parseInt(burstProp.trim());
                    if (parsed >= 32 && parsed <= 4096) {
                        burst = parsed;
                    }
                }
            } catch (NumberFormatException ignored) {
            }
        }
        return new int[]{sr, burst};
    }

    private void startAudioInternal() {
        synchronized (streamLock) {
            if (!serviceAlive || !wantAudioRunning) {
                return;
            }
            int[] hints = queryAudioHints();
            int res = Native.start(hints[0], hints[1]);
            if (res == 0) {
                lastHealthyStartMs = SystemClock.elapsedRealtime();
            }
        }
    }

    void retryAudioAsync() {
        wantAudioRunning = true;
        synchronized (streamLock) {
            consecutiveRecoveries = 0;
        }
        workers.submit(() -> {
            synchronized (streamLock) {
                Native.stop();
                startAudioInternal();
            }
        });
    }

    void stopAudioAsync() {
        wantAudioRunning = false;
        workers.submit(() -> {
            synchronized (streamLock) {
                Native.stop();
            }
        });
    }

    private void monitorAudioHealth() {
        if (!serviceAlive || !wantAudioRunning) {
            return;
        }
        int err = Native.healthy();
        if (err == 0) {
            if (lastHealthyStartMs > 0L
                    && SystemClock.elapsedRealtime() - lastHealthyStartMs > 3000L) {
                consecutiveRecoveries = 0;
            }
            return;
        }
        if (!Native.isReconnectableError(err)) {
            return;
        }
        long now = SystemClock.elapsedRealtime();
        synchronized (streamLock) {
            if (!serviceAlive || !wantAudioRunning) {
                return;
            }
            if (now - lastRecoveryMs < 1000L || consecutiveRecoveries >= 4) {
                return;
            }
            lastRecoveryMs = now;
            consecutiveRecoveries++;
            Native.stop();
            startAudioInternal();
        }
    }

    String dispatchFromUi(String cmdJson) {
        String reply = Native.dispatchJson(cmdJson);
        try {
            JSONObject obj = new JSONObject(cmdJson);
            if ("hostControl".equals(obj.optString("type"))) {
                String op = obj.optString("op");
                if ("toggleDevBridge".equals(op)) {
                    boolean active = new JSONObject(Native.diagnosticsJson())
                            .optBoolean("devBridgeEnabled", false);
                    setDevBridgeEnabled(active);
                } else if ("retryAudio".equals(op)
                        || "startAudio".equals(op)
                        || "reconnectAudio".equals(op)) {
                    retryAudioAsync();
                } else if ("stopAudio".equals(op)) {
                    stopAudioAsync();
                }
            }
        } catch (Exception ignored) {
        }
        return reply;
    }

    private synchronized void setDevBridgeEnabled(boolean enabled) {
        if (enabled == devBridgeEnabled) {
            return;
        }
        devBridgeEnabled = enabled;
        if (!enabled) {
            closeDevServer();
            return;
        }
        try {
            devServer = new ServerSocket(8787);
            final ServerSocket serverRef = devServer;
            workers.submit(() -> {
                while (serviceAlive && devBridgeEnabled && devServer == serverRef) {
                    try {
                        Socket s = serverRef.accept();
                        s.setSoTimeout(10000);
                        if (sockets.size() >= 4) {
                            s.close();
                            continue;
                        }
                        sockets.add(s);
                        workers.submit(() -> serveDevClient(s));
                    } catch (IOException e) {
                        if (!devBridgeEnabled || !serviceAlive) {
                            break;
                        }
                    }
                }
            });
        } catch (IOException ignored) {
            devBridgeEnabled = false;
            closeDevServer();
        }
    }

    private synchronized void closeDevServer() {
        if (devServer != null) {
            try {
                devServer.close();
            } catch (IOException ignored) {
            }
            devServer = null;
        }
        for (Socket s : sockets) {
            try {
                s.close();
            } catch (IOException ignored) {
            }
        }
        sockets.clear();
    }

    String describeDevBridgeStatus() {
        if (!devBridgeEnabled) {
            return "Route: " + currentAudioRoute + " · Developer LAN bridge: OFF (opt-in)";
        }
        return "Route: " + currentAudioRoute + " · Developer LAN bridge: http://" + bestLanIp()
                + ":8787 (Bearer Header Auth ONLY; token=" + cachedToken + ")";
    }

    private static String bestLanIp() {
        try {
            Enumeration<NetworkInterface> nics = NetworkInterface.getNetworkInterfaces();
            while (nics.hasMoreElements()) {
                Enumeration<InetAddress> addrs = nics.nextElement().getInetAddresses();
                while (addrs.hasMoreElements()) {
                    InetAddress addr = addrs.nextElement();
                    if (addr instanceof Inet4Address && !addr.isLoopbackAddress()) {
                        return addr.getHostAddress();
                    }
                }
            }
        } catch (Exception ignored) {
        }
        return "127.0.0.1";
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null) {
            String action = intent.getAction();
            if ("STOP".equals(action)) {
                stopSelf();
            } else if ("RETRY_AUDIO".equals(action)) {
                retryAudioAsync();
            }
        }
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return binder;
    }

    @Override
    public synchronized void onDestroy() {
        serviceAlive = false;
        wantAudioRunning = false;
        devBridgeEnabled = false;
        closeDevServer();
        scheduler.shutdownNow();
        workers.shutdownNow();

        if (audioManager != null && audioDeviceCallback != null) {
            audioManager.unregisterAudioDeviceCallback(audioDeviceCallback);
        }
        if (midiManager != null && midiDeviceCallback != null) {
            midiManager.unregisterDeviceCallback(midiDeviceCallback);
        }
        for (OpenMidiDevice dev : openMidiDevices.values()) {
            dev.close();
        }
        openMidiDevices.clear();

        Native.stop();
        if (wakeLock != null && wakeLock.isHeld()) {
            wakeLock.release();
        }
        if (audioManager != null && focusRequest != null) {
            audioManager.abandonAudioFocusRequest(focusRequest);
        }
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }

    private static byte[] readBytes(InputStream in, int length) throws IOException {
        byte[] out = new byte[length];
        int pos = 0;
        while (pos < length) {
            int n = in.read(out, pos, length - pos);
            if (n < 0) {
                throw new EOFException();
            }
            pos += n;
        }
        return out;
    }

    private static byte[] readAsset(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] chunk = new byte[8192];
        int n;
        while ((n = in.read(chunk)) != -1) {
            out.write(chunk, 0, n);
            if (out.size() > 8 * 1024 * 1024) {
                throw new IOException("asset size");
            }
        }
        return out.toByteArray();
    }

    private static String line(InputStream in) throws IOException {
        ByteArrayOutputStream b = new ByteArrayOutputStream();
        for (int i = 0; i < 8192; i++) {
            int c = in.read();
            if (c == -1) {
                throw new EOFException();
            }
            if (c == '\n') {
                return b.toString("UTF-8").trim();
            }
            b.write(c);
        }
        throw new IOException("header too long");
    }

    private static void response(OutputStream out, int code, String type, byte[] data) throws IOException {
        String reason = (code == 200) ? "OK" : (code == 202 ? "Accepted" : (code == 401 ? "Unauthorized" : "Error"));
        String hdr = "HTTP/1.1 " + code + " " + reason
                + "\r\nContent-Type: " + type
                + "\r\nContent-Length: " + data.length
                + "\r\nConnection: close"
                + "\r\nCache-Control: no-store\r\n\r\n";
        out.write(hdr.getBytes(StandardCharsets.US_ASCII));
        out.write(data);
        out.flush();
    }

    private void serveDevClient(Socket socket) {
        try (Socket s = socket) {
            InputStream in = s.getInputStream();
            OutputStream out = s.getOutputStream();
            String[] request = line(in).split(" ");
            if (request.length < 2) {
                return;
            }
            URI uri = URI.create(request[1]);
            String path = uri.getPath();
            Map<String, String> headers = new HashMap<>();
            int total = 0;
            for (;;) {
                String h = line(in);
                total += h.length();
                if (total > 16384) {
                    throw new IOException("headers too long");
                }
                if (h.isEmpty()) {
                    break;
                }
                int c = h.indexOf(':');
                if (c > 0) {
                    headers.put(h.substring(0, c).toLowerCase(Locale.ROOT), h.substring(c + 1).trim());
                }
            }
            // Reject query-string tokens strictly; authenticate only via Authorization: Bearer header.
            String rawQuery = uri.getRawQuery();
            if (rawQuery != null && rawQuery.contains("token=")) {
                response(out, 401, "application/json",
                        "{\"error\":\"Query string tokens are prohibited; use Authorization: Bearer\"}".getBytes(StandardCharsets.UTF_8));
                return;
            }
            boolean auth = ("Bearer " + cachedToken).equals(headers.get("authorization"));
            if (path.startsWith("/api/") && !auth) {
                response(out, 401, "application/json", "{\"error\":\"unauthorized\"}".getBytes(StandardCharsets.UTF_8));
                return;
            }
            if ("/api/v1/ws".equals(path)) {
                String key = headers.get("sec-websocket-key");
                if (key == null) {
                    return;
                }
                String accept = android.util.Base64.encodeToString(
                        MessageDigest.getInstance("SHA-1")
                                .digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").getBytes(StandardCharsets.US_ASCII)),
                        android.util.Base64.NO_WRAP);
                out.write(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: "
                        + accept + "\r\n\r\n").getBytes(StandardCharsets.US_ASCII));
                out.flush();
                while (serviceAlive && devBridgeEnabled) {
                    if (in.available() > 0) {
                        int first = in.read();
                        int len = in.read();
                        if (first < 0 || len < 0 || (first & 15) == 8) {
                            break;
                        }
                        int n = len & 127;
                        if (n == 126) {
                            n = (in.read() << 8) | in.read();
                        } else if (n == 127) {
                            break;
                        }
                        int skip = n + ((len & 128) != 0 ? 4 : 0);
                        while (skip-- > 0) {
                            if (in.read() < 0) {
                                throw new EOFException();
                            }
                        }
                    }
                    byte[] data = Native.pollSnapshotJson().getBytes(StandardCharsets.UTF_8);
                    out.write(0x81);
                    if (data.length < 126) {
                        out.write(data.length);
                    } else {
                        out.write(126);
                        out.write(data.length >> 8);
                        out.write(data.length & 255);
                    }
                    out.write(data);
                    out.flush();
                    Thread.sleep(40L);
                }
                return;
            }
            if ("/api/v1/snapshot".equals(path)) {
                response(out, 200, "application/json",
                        Native.pollSnapshotJson().getBytes(StandardCharsets.UTF_8));
                return;
            }
            if ("/api/v1/diagnostics".equals(path)) {
                response(out, 200, "application/json",
                        Native.diagnosticsJson().getBytes(StandardCharsets.UTF_8));
                return;
            }
            if ("/api/v1/command".equals(path) && "POST".equals(request[0])) {
                int length = Integer.parseInt(headers.getOrDefault("content-length", "0"));
                if (length < 1 || length > 65536) {
                    throw new IOException("body size");
                }
                byte[] data = readBytes(in, length);
                String reply = dispatchFromUi(new String(data, StandardCharsets.UTF_8));
                boolean ok = new JSONObject(reply).optBoolean("ok", false);
                response(out, ok ? 200 : 400, "application/json", reply.getBytes(StandardCharsets.UTF_8));
                return;
            }
            if (path.startsWith("/api/")) {
                response(out, 404, "application/json",
                        "{\"error\":\"unsupported endpoint\"}".getBytes(StandardCharsets.UTF_8));
                return;
            }
            if (path.contains("..")) {
                return;
            }
            String asset = "/".equals(path) ? "index.html" : path.substring(1);
            try (InputStream file = getAssets().open(asset)) {
                String type = asset.endsWith(".js") ? "application/javascript"
                        : asset.endsWith(".css") ? "text/css"
                        : asset.endsWith(".html") ? "text/html" : "application/octet-stream";
                response(out, 200, type, readAsset(file));
            }
        } catch (Exception ignored) {
        } finally {
            sockets.remove(socket);
        }
    }
}
