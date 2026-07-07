// day-part-sensors' OWN Android backend — a headless capability shim (no UI). It is bundled with this
// crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO edits to
// day-android; it registers no view. Android sensors are push-only, so the shim lazily registers a
// SensorEventListener per sensor on the first read() and caches the newest sample for Rust to poll.
// No manifest permission is needed for these sensors at SENSOR_DELAY_UI rates. It uses day-android's
// public Context (DayBridge.ctx). It is the Android twin of parts/day-part-sensors/src/*.rs's other
// per-OS impls.
package dev.daybrite.day.sensors;

import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;

import dev.daybrite.day.bridge.DayBridge;

public final class DaySensors {
    private DaySensors() {}

    /** Kind codes shared with src/android.rs: 0=accelerometer, 1=gyroscope, 2=magnetometer. */
    private static final int KIND_COUNT = 3;

    private static final Object lock = new Object();
    private static final SensorEventListener[] listeners = new SensorEventListener[KIND_COUNT];
    private static final float[][] latest = new float[KIND_COUNT][]; // null until the first event

    private static int sensorType(int kind) {
        switch (kind) {
            case 0:
                return Sensor.TYPE_ACCELEROMETER; // m/s², includes gravity
            case 1:
                return Sensor.TYPE_GYROSCOPE; // rad/s
            case 2:
                return Sensor.TYPE_MAGNETIC_FIELD; // µT
            default:
                return -1;
        }
    }

    private static SensorManager manager() {
        Context ctx = DayBridge.ctx;
        return ctx == null ? null : (SensorManager) ctx.getSystemService(Context.SENSOR_SERVICE);
    }

    /** Whether the device has the given sensor. */
    public static boolean isAvailable(int kind) {
        int type = sensorType(kind);
        SensorManager sm = manager();
        return sm != null && type >= 0 && sm.getDefaultSensor(type) != null;
    }

    /**
     * The latest sample as {x, y, z} (SI units, per the sensor type), or null when the sensor is
     * missing or no event has arrived yet. The first call registers a listener at SENSOR_DELAY_UI,
     * kept for the process lifetime.
     */
    public static double[] read(int kind) {
        int type = sensorType(kind);
        SensorManager sm = manager();
        if (sm == null || type < 0) {
            return null;
        }
        synchronized (lock) {
            if (listeners[kind] == null) {
                Sensor sensor = sm.getDefaultSensor(type);
                if (sensor == null) {
                    return null;
                }
                final int k = kind;
                SensorEventListener listener = new SensorEventListener() {
                    @Override
                    public void onSensorChanged(SensorEvent event) {
                        synchronized (lock) {
                            latest[k] = new float[] {
                                event.values[0], event.values[1], event.values[2]
                            };
                        }
                    }

                    @Override
                    public void onAccuracyChanged(Sensor s, int accuracy) {}
                };
                sm.registerListener(listener, sensor, SensorManager.SENSOR_DELAY_UI);
                listeners[kind] = listener;
            }
            float[] v = latest[kind];
            return v == null ? null : new double[] {v[0], v[1], v[2]};
        }
    }
}
