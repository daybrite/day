// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.clipboard;

import android.content.*;
import android.content.res.AssetFileDescriptor;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.*;
import android.provider.OpenableColumns;
import java.io.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Read-only, per-app URI grants: clipboard bytes never travel as a large Binder parcel. */
public final class BinaryClipboardProvider extends ContentProvider {
    private static final int LIMIT = 64 * 1024 * 1024;
    @Override public boolean onCreate() { return true; }
    private static File root(Context c) { return new File(c.getCacheDir(), "day-clipboard"); }
    private File folder(Uri uri) throws FileNotFoundException {
        String id = uri.getLastPathSegment();
        if (uri.getPathSegments().size() != 1 || id == null || !id.matches("[a-f0-9-]{36}")) throw new FileNotFoundException();
        File f = new File(root(getContext()), id);
        if (!f.isDirectory()) throw new FileNotFoundException();
        return f;
    }
    private String[] types(Uri uri) throws IOException {
        byte[] bytes = readBytes(new FileInputStream(new File(folder(uri), "types")), 8192);
        return new String(bytes, StandardCharsets.UTF_8).split("\n");
    }
    @Override public String getType(Uri uri) { try { return types(uri)[0]; } catch (IOException e) { return null; } }
    @Override public String[] getStreamTypes(Uri uri, String filter) {
        try { return Arrays.stream(types(uri)).filter(t -> ClipDescription.compareMimeTypes(t, filter)).toArray(String[]::new); }
        catch (IOException e) { return null; }
    }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("read-only clipboard");
        return ParcelFileDescriptor.open(new File(folder(uri), "0"), ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public AssetFileDescriptor openTypedAssetFile(Uri uri, String mime, Bundle opts) throws FileNotFoundException {
        try {
            String[] types = types(uri);
            for (int i = 0; i < types.length; i++) if (ClipDescription.compareMimeTypes(types[i], mime)) {
                File f = new File(folder(uri), String.valueOf(i));
                return new AssetFileDescriptor(ParcelFileDescriptor.open(f, ParcelFileDescriptor.MODE_READ_ONLY), 0, f.length());
            }
        } catch (IOException e) { throw new FileNotFoundException(e.toString()); }
        throw new FileNotFoundException("representation unavailable");
    }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String sort) {
        MatrixCursor out = new MatrixCursor(new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        try { File f = new File(folder(uri), "0"); out.addRow(new Object[]{"clipboard", f.length()}); } catch (IOException ignored) {}
        return out;
    }
    @Override public Uri insert(Uri u, ContentValues v) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri u, ContentValues v, String s, String[] a) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri u, String s, String[] a) { throw new UnsupportedOperationException(); }
    private static byte[] readBytes(InputStream input, int limit) throws IOException {
        try (InputStream in = input; ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buf = new byte[8192]; int n;
            while ((n = in.read(buf)) != -1) { if (out.size() + n > limit) throw new IOException("clipboard too large"); out.write(buf,0,n); }
            return out.toByteArray();
        }
    }
    public static boolean write(Context ctx, byte[] packet) {
        if (ctx == null) return false;
        try {
            ByteBuffer data = ByteBuffer.wrap(packet).order(ByteOrder.LITTLE_ENDIAN);
            int count = data.getInt(); if (count < 1 || count > 32) return false;
            File folder = new File(root(ctx), UUID.randomUUID().toString()); if (!folder.mkdirs()) return false;
            String[] types = new String[count];
            for (int i = 0; i < count; i++) {
                int m = data.getInt(), n = data.getInt();
                if (m < 1 || m > 255 || n < 0 || n > LIMIT || (long)m+n > data.remaining()) return false;
                byte[] name = new byte[m]; data.get(name); types[i] = new String(name, StandardCharsets.UTF_8);
                try (OutputStream out = new FileOutputStream(new File(folder, String.valueOf(i)))) { out.write(packet, data.position(), n); }
                data.position(data.position()+n);
            }
            try (OutputStream out = new FileOutputStream(new File(folder, "types"))) { out.write(String.join("\n",types).getBytes(StandardCharsets.UTF_8)); }
            Uri uri = new Uri.Builder().scheme("content").authority(ctx.getPackageName()+".day.clipboard").appendPath(folder.getName()).build();
            ((ClipboardManager)ctx.getSystemService(Context.CLIPBOARD_SERVICE)).setPrimaryClip(new ClipData("day", types, new ClipData.Item(uri)));
            return true;
        } catch (Exception e) { return false; }
    }
    public static byte[] read(Context ctx, String preferred) {
        try {
            ClipboardManager cm = (ClipboardManager)ctx.getSystemService(Context.CLIPBOARD_SERVICE);
            ClipData clip = cm.getPrimaryClip();
            if (clip == null || clip.getItemCount() == 0) return new byte[4];
            ClipData.Item item = clip.getItemAt(0);
            for (String mime : preferred.split("\n")) {
                byte[] bytes = null;
                if (mime.equals("text/plain") && item.getText() != null) bytes = item.getText().toString().getBytes(StandardCharsets.UTF_8);
                else if (item.getUri() != null && clip.getDescription().hasMimeType(mime)) {
                    try (AssetFileDescriptor fd = ctx.getContentResolver().openTypedAssetFileDescriptor(item.getUri(), mime, null)) {
                        if (fd != null) bytes = readBytes(fd.createInputStream(), LIMIT);
                    }
                }
                if (bytes != null) {
                    byte[] name = mime.getBytes(StandardCharsets.UTF_8);
                    return ByteBuffer.allocate(12+name.length+bytes.length).order(ByteOrder.LITTLE_ENDIAN).putInt(1).putInt(name.length).putInt(bytes.length).put(name).put(bytes).array();
                }
            }
        } catch (Exception e) { return new byte[0]; }
        return new byte[4];
    }
}
