// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.sensors;

import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.os.Handler;
import android.os.Looper;
import dev.daybrite.day.bridge.DayBridge;

public final class DaySensors {
    private DaySensors() {}
    private static final SensorEventListener[] listeners = new SensorEventListener[6];
    private static final SensorManager[] managers = new SensorManager[6];
    private static native void nativeSample(int kind, int generation, double x, double y, double z);
    private static int sensorType(int kind) {
        switch (kind) {
            case 0: return Sensor.TYPE_ACCELEROMETER;
            case 1: return Sensor.TYPE_GYROSCOPE;
            case 2: return Sensor.TYPE_MAGNETIC_FIELD;
            case 3: return android.os.Build.VERSION.SDK_INT >= 26
                ? Sensor.TYPE_ACCELEROMETER_UNCALIBRATED : -1;
            case 4: return Sensor.TYPE_GYROSCOPE_UNCALIBRATED;
            case 5: return Sensor.TYPE_MAGNETIC_FIELD_UNCALIBRATED;
            default: return -1;
        }
    }
    private static SensorManager manager() {
        Context ctx = DayBridge.ctx;
        return ctx == null ? null : (SensorManager) ctx.getSystemService(Context.SENSOR_SERVICE);
    }
    public static boolean isAvailable(int kind) {
        SensorManager sm = manager();
        int type = sensorType(kind);
        return sm != null && type >= 0 && sm.getDefaultSensor(type) != null;
    }
    // Lifecycle calls are serialized by the Rust dispatcher. Events enqueue bounded data only.
    public static boolean start(int kind, int generation) {
        SensorManager sm = manager();
        int type = sensorType(kind);
        if (sm == null || type < 0) return false;
        Sensor sensor = sm.getDefaultSensor(type);
        if (sensor == null) return false;
        SensorEventListener listener = new SensorEventListener() {
            @Override public void onSensorChanged(SensorEvent event) {
                nativeSample(kind, generation, event.values[0], event.values[1], event.values[2]);
            }
            @Override public void onAccuracyChanged(Sensor sensor, int accuracy) {}
        };
        if (!sm.registerListener(listener, sensor, 50_000, new Handler(Looper.getMainLooper()))) return false;
        listeners[kind] = listener;
        managers[kind] = sm;
        return true;
    }
    public static void stop(int kind) {
        if (kind < 0 || kind >= listeners.length || listeners[kind] == null) return;
        managers[kind].unregisterListener(listeners[kind]);
        listeners[kind] = null;
        managers[kind] = null;
    }
}
