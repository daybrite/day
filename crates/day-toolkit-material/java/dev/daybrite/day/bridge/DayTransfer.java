// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.bridge;
import android.app.Activity;
import android.content.*;
import android.content.res.AssetFileDescriptor;
import android.view.*;
import android.net.Uri;
import android.os.CancellationSignal;
import java.util.concurrent.atomic.AtomicReference;
import java.io.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
/** Android system drag sessions with scoped read grants. Never touches ClipboardManager. */
public final class DayTransfer {
    static final int LIMIT=64*1024*1024;
    static final String BUNDLE="application/vnd.day.transfer";
    static final Map<View,State> states=new WeakHashMap<>();
    static final ExecutorService reads=Executors.newSingleThreadExecutor();
    static final class State {long token; boolean alive=true;float x,y;}
    private static native byte[] nativeCall(int kind,long token,double x,double y,byte[] bytes,boolean local);
    private static State state(View v,long token) {State s=states.get(v);if(s==null){s=new State();s.token=token;states.put(v,s);}return s;}
    public static void release(View v) {State s=states.remove(v);if(s!=null)s.alive=false;}
    public static void source(View v,long token) {
        State s=state(v,token);
        v.setOnTouchListener((view,event)->{s.x=event.getX();s.y=event.getY();return false;});
        v.setOnLongClickListener(view->{
            if(!s.alive)return false;
            float density=view.getResources().getDisplayMetrics().density;
            byte[] packet=nativeCall(0,token,s.x/density,s.y/density,new byte[0],true);
            if(packet==null||packet.length==0)return false;
            try {return view.startDragAndDrop(DayTransferProvider.publish(view.getContext(),packet),new View.DragShadowBuilder(view),s,View.DRAG_FLAG_GLOBAL|View.DRAG_FLAG_GLOBAL_URI_READ);}
            catch(Exception error){android.util.Log.w("DayTransfer", "Unable to start drag", error);return false;}
        });
    }
    private static Activity activity(Context context) {while(context instanceof ContextWrapper){if(context instanceof Activity)return (Activity)context;context=((ContextWrapper)context).getBaseContext();}return null;}
    private static boolean accept(View v,State s,DragEvent e,String types) {
        float density=v.getResources().getDisplayMetrics().density;
        byte[] result=nativeCall(1,s.token,e.getX()/density,e.getY()/density,types.getBytes(StandardCharsets.UTF_8),e.getLocalState() instanceof State);
        return s.alive&&result!=null&&result.length==1&&result[0]==1;
    }
    private static byte[] read(InputStream in) throws IOException {
        try(InputStream input=in;ByteArrayOutputStream out=new ByteArrayOutputStream()) {byte[] b=new byte[65536];int n;while((n=input.read(b))!=-1){if(out.size()+n>LIMIT)throw new IOException("transfer too large");out.write(b,0,n);}return out.toByteArray();}
    }
    private static byte[] packet(String mime,byte[] bytes) {
        byte[] m=mime.getBytes(StandardCharsets.UTF_8);
        return ByteBuffer.allocate(24+m.length+bytes.length).order(ByteOrder.LITTLE_ENDIAN).put(new byte[]{68,65,89,68,78,68,0,1}).putInt(1).putInt(1).putInt(m.length).putInt(bytes.length).put(m).put(bytes).array();
    }
    private static byte[] receive(ContentResolver resolver, ClipData clip,
                                  ClipDescription description, CancellationSignal cancel,
                                  AtomicReference<AssetFileDescriptor> opened) throws IOException {
        ArrayList<byte[]> items = new ArrayList<>();
        int total = 12;
        for (int i = 0; i < clip.getItemCount(); i++) {
            cancel.throwIfCanceled();
            ClipData.Item item = clip.getItemAt(i);
            Uri uri = item.getUri();
            byte[] data;
            if (uri != null) {
                String mime = description.hasMimeType(BUNDLE) ? BUNDLE : resolver.getType(uri);
                if (mime == null) mime = "application/octet-stream";
                // A Day bundle already contains every item. Mixing it with additional
                // ClipData items would be ambiguous; reject instead of silently losing data.
                if (mime.equals(BUNDLE) && clip.getItemCount() != 1) return null;
                try (AssetFileDescriptor fd = resolver.openTypedAssetFileDescriptor(uri, mime, null, cancel)) {
                    if (fd == null) return null;
                    opened.set(fd);
                    cancel.throwIfCanceled();
                    byte[] bytes = read(fd.createInputStream());
                    if (mime.equals(BUNDLE)) return bytes;
                    data = packet(mime, bytes);
                } finally { opened.set(null); }
            } else if (item.getText() != null) {
                data = packet("text/plain", item.getText().toString().getBytes(StandardCharsets.UTF_8));
            } else return null;
            total += data.length - 12;
            if (total > LIMIT) throw new IOException("transfer too large");
            items.add(data);
        }
        ByteBuffer out = ByteBuffer.allocate(total).order(ByteOrder.LITTLE_ENDIAN);
        out.put(new byte[]{68,65,89,68,78,68,0,1}).putInt(items.size());
        for (byte[] item : items) out.put(item, 12, item.length - 12);
        return out.array();
    }
    public static void target(View v,long token) {
        State s=state(v,token);
        v.setOnDragListener((view,event)->{
            if(!s.alive)return false;
            ClipDescription description=event.getClipDescription();StringBuilder types=new StringBuilder();
            if(description!=null)for(int i=0;i<description.getMimeTypeCount();i++)types.append(description.getMimeType(i)).append('\n');
            switch(event.getAction()) {
                // Subscribe by types, not coordinates: rejecting STARTED loses later LOCATION.
                case DragEvent.ACTION_DRAG_STARTED:return description!=null;
                case DragEvent.ACTION_DRAG_ENTERED:
                case DragEvent.ACTION_DRAG_LOCATION:return accept(view,s,event,types.toString());
                case DragEvent.ACTION_DRAG_EXITED:
                case DragEvent.ACTION_DRAG_ENDED:return true;
                case DragEvent.ACTION_DROP:
                    if(!accept(view,s,event,types.toString()))return false;
                    final ClipData clip=event.getClipData();if(clip==null||clip.getItemCount()<1||clip.getItemCount()>256)return false;
                    Activity a=activity(view.getContext());
                    final DragAndDropPermissions permission=a==null?null:a.requestDragAndDropPermissions(event);
                    final float density=view.getResources().getDisplayMetrics().density;
                    final double x=event.getX()/density,y=event.getY()/density;
                    final boolean local=event.getLocalState() instanceof State;
                    final java.util.concurrent.atomic.AtomicBoolean finished=new java.util.concurrent.atomic.AtomicBoolean();
                    final ContentResolver resolver=view.getContext().getContentResolver();
                    final CancellationSignal cancel = new CancellationSignal();
                    final AtomicReference<AssetFileDescriptor> opened = new AtomicReference<>();
                    Future<?> pending=reads.submit(()->{
                        byte[] payload=null;
                        try { payload=receive(resolver,clip,description,cancel,opened); }
                        catch(Exception error) {android.util.Log.w("DayTransfer", "Unable to read drop", error);}
                        final byte[] result=payload;
                        DayBridge.main.post(()->{if(!finished.compareAndSet(false,true))return;try{if(s.alive&&result!=null)nativeCall(2,token,x,y,result,local);}finally{if(permission!=null)permission.release();}});
                    });
                    DayBridge.main.postDelayed(()->{
                        if(finished.compareAndSet(false,true)){
                            cancel.cancel();
                            AssetFileDescriptor fd=opened.getAndSet(null);
                            if(fd!=null)try{fd.close();}catch(IOException ignored){}
                            pending.cancel(true);
                            if(permission!=null)permission.release();
                        }
                    },30000);
                    return true;
                default:return false;
            }
        });
    }
}
