// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

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

/** Read-only, per-app URI grants: transfer bytes never travel as a large Binder parcel. */
public final class DayTransferProvider extends ContentProvider {
    private static final int LIMIT = 64 * 1024 * 1024;
    @Override public boolean onCreate() { return true; }
    private static File root(Context c) { return new File(c.getCacheDir(), "day-transfer"); }
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
    @Override public String getType(Uri uri) { try { return types(uri)[1]; } catch (IOException e) { return null; } }
    @Override public String[] getStreamTypes(Uri uri, String filter) {
        try { return Arrays.stream(types(uri)).filter(t -> ClipDescription.compareMimeTypes(t, filter)).toArray(String[]::new); }
        catch (IOException e) { return null; }
    }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new FileNotFoundException("read-only transfer");
        return ParcelFileDescriptor.open(new File(folder(uri), "1"), ParcelFileDescriptor.MODE_READ_ONLY);
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
        try { File f = new File(folder(uri), "1"); out.addRow(new Object[]{"transfer", f.length()}); } catch (IOException ignored) {}
        return out;
    }
    @Override public Uri insert(Uri u, ContentValues v) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri u, ContentValues v, String s, String[] a) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri u, String s, String[] a) { throw new UnsupportedOperationException(); }
    private static byte[] readBytes(InputStream input, int limit) throws IOException {
        try (InputStream in = input; ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buf = new byte[8192]; int n;
            while ((n = in.read(buf)) != -1) { if (out.size() + n > limit) throw new IOException("transfer too large"); out.write(buf,0,n); }
            return out.toByteArray();
        }
    }
    public static ClipData publish(Context ctx, byte[] packet) throws IOException {
        ByteBuffer data=ByteBuffer.wrap(packet).order(ByteOrder.LITTLE_ENDIAN);
        if(packet.length>LIMIT || packet.length<16) throw new IOException("invalid transfer");
        data.position(12);int count=data.getInt();if(count<1||count>32)throw new IOException("invalid transfer");
        File folder=new File(root(ctx),UUID.randomUUID().toString());if(!folder.mkdirs())throw new IOException("cache unavailable");
        ArrayList<String> types=new ArrayList<>();types.add("application/vnd.day.transfer");
        try(OutputStream out=new FileOutputStream(new File(folder,"0"))) {out.write(packet);}
        for(int i=0;i<count;i++) {
            int m=data.getInt(),n=data.getInt();if(m<1||m>255||n<0||(long)m+n>data.remaining())throw new IOException("invalid transfer");
            byte[] name=new byte[m];data.get(name);types.add(new String(name,StandardCharsets.UTF_8));
            try(OutputStream out=new FileOutputStream(new File(folder,String.valueOf(i+1)))) {out.write(packet,data.position(),n);}data.position(data.position()+n);
        }
        try(OutputStream out=new FileOutputStream(new File(folder,"types"))) {out.write(String.join("\n",types).getBytes(StandardCharsets.UTF_8));}
        Uri uri=new Uri.Builder().scheme("content").authority(ctx.getPackageName()+".day.transfer").appendPath(folder.getName()).build();
        // Pending destination reads have a 30-second timeout. Open descriptors remain valid
        // when these cached files are unlinked after the much longer publication lease.
        new Handler(Looper.getMainLooper()).postDelayed(()->{File[] files=folder.listFiles();if(files!=null)for(File f:files)f.delete();folder.delete();},300000);
        return new ClipData("Day transfer",types.toArray(new String[0]),new ClipData.Item(uri));
    }
}
