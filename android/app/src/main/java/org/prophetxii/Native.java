package org.prophetxii;

final class Native {
    static {
        System.loadLibrary("prophet_android");
    }

    private Native() {}

    static native int start(int hintSampleRate, int hintFramesPerBurst);
    static native void stop();
    static native int healthy();
    static native boolean isReconnectableError(int errorCode);
    static native String dispatchJson(String cmdJson);
    static native String pollSnapshotJson();
    static native String diagnosticsJson();
    static native int usbMidi(byte[] data, int offset, int length);
    static native void setFocusGain(String state, boolean muted, float gain);
    static native void setRouteInfo(String route, String usbDevicesCsv);
}
