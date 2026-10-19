// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.view.View;
import android.view.ViewTreeObserver;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

/** Window-local editing lifetime. Never dismiss between editors or on ordinary button taps.
 *  Android does not hide the IME when focus moves to a non-editor or an editor is removed.
 *  Reconcile after the current UI turn, so a replacement editor/field chain can take focus.
 *  Navigation also calls endEditing before detaching the outgoing surface. */
final class DayKeyboard implements ViewTreeObserver.OnGlobalFocusChangeListener,
        ViewTreeObserver.OnGlobalLayoutListener, View.OnAttachStateChangeListener {
    private final View root;
    private java.lang.ref.WeakReference<View> editor = new java.lang.ref.WeakReference<>(null);
    private ViewTreeObserver observer;
    private boolean pending;
    // IME show/hide is asynchronous. A show already in flight can land after focus loss;
    // retain this intent until another editor takes focus and recheck after inset layout.
    private boolean editingEnded;

    static void install(View root) {
        DayKeyboard keyboard = new DayKeyboard(root);
        root.addOnAttachStateChangeListener(keyboard);
        if (root.isAttachedToWindow()) keyboard.onViewAttachedToWindow(root);
    }

    private DayKeyboard(View root) { this.root = root; }

    private static boolean editable(View view) {
        // A WebView may own an HTML editor without exposing a native EditText.
        return view != null && (view.onCheckIsTextEditor() || view instanceof android.webkit.WebView);
    }

    private static boolean activeEditor(View view) {
        return editable(view) && view.isAttachedToWindow() && view.isShown() && view.isEnabled();
    }

    private static void hide(View root) {
        WindowInsetsControllerCompat controller = ViewCompat.getWindowInsetsController(root);
        if (controller != null) controller.hide(WindowInsetsCompat.Type.ime());
    }

    /** Only resign an editor inside this outgoing subtree, never a sibling/new destination. */
    static void endEditing(View subtree) {
        if (subtree == null) return;
        View focused = subtree.findFocus();
        if (!editable(focused)) return;
        // The attached window is still available here; after removeView its token may be gone.
        hide(subtree.getRootView());
        focused.clearFocus();
    }

    @Override public void onViewAttachedToWindow(View view) {
        observer = root.getViewTreeObserver();
        observer.addOnGlobalFocusChangeListener(this);
        observer.addOnGlobalLayoutListener(this);
        if (activeEditor(root.findFocus())) editor = new java.lang.ref.WeakReference<>(root.findFocus());
    }

    @Override public void onViewDetachedFromWindow(View view) {
        if (observer != null && observer.isAlive()) {
            observer.removeOnGlobalFocusChangeListener(this);
            observer.removeOnGlobalLayoutListener(this);
        }
        observer = null;
        editor.clear();
        editingEnded = false;
    }

    @Override public void onGlobalFocusChanged(View oldFocus, View newFocus) {
        if (editable(oldFocus)) {
            editor = new java.lang.ref.WeakReference<>(oldFocus);
            editingEnded = true;
        }
        if (activeEditor(newFocus)) {
            editor = new java.lang.ref.WeakReference<>(newFocus);
            editingEnded = false;
        }
        reconcileLater();
    }

    @Override public void onGlobalLayout() {
        // Covers a focused editor/ancestor becoming hidden or disabled without a focus event.
        View previous = editor.get();
        if (previous != null && !activeEditor(previous)) reconcileLater();
        WindowInsetsCompat insets = ViewCompat.getRootWindowInsets(root);
        if (editingEnded && insets != null && insets.isVisible(WindowInsetsCompat.Type.ime())) {
            reconcileLater();
        }
    }

    private void reconcileLater() {
        if (pending || (editor.get() == null && !editingEnded)) return;
        pending = true;
        root.post(() -> {
            pending = false;
            // A popup, permission dialog or another activity owns focus: do not hide its IME.
            if (!root.isAttachedToWindow() || !root.hasWindowFocus()) return;
            View current = root.findFocus();
            if (activeEditor(current)) {
                editor = new java.lang.ref.WeakReference<>(current);
                editingEnded = false;
                return;
            }
            if (editor.get() == null && !editingEnded) return;
            editor.clear();
            editingEnded = true;
            hide(root);
            if (editable(current)) current.clearFocus();
        });
    }
}
