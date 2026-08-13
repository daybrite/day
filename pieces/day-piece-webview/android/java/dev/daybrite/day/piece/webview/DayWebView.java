// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-piece-webview crate's OWN Android backend — bundled here and folded into the app's Gradle
// build via [package.metadata.day.android], with ZERO edits to day-android. It uses only day-android's
// PUBLIC Java surface: DayBridge.ctx (the Context) and DayBridge.nativeOnEvent (the event trampoline).
// The piece also declares its INTERNET permission in Cargo.toml, which `day build` merges into the app
// manifest — so a WebView-using app needs no manual manifest edit. See docs/extending.md.
package dev.daybrite.day.piece.webview;

import android.view.View;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import dev.daybrite.day.bridge.DayBridge;

/** Wraps android.webkit.WebView, reporting the finished URL back via the open Custom-event kind (12). */
public final class DayWebView {
    private DayWebView() {}

    public static View makeWebView(long id, String url, String inlinePrefix) {
        WebView web = new WebView(DayBridge.ctx);
        web.getSettings().setJavaScriptEnabled(true);
        web.getSettings().setDomStorageEnabled(true);
        final boolean inline = inlinePrefix != null && !inlinePrefix.isEmpty();
        web.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageFinished(WebView view, String finishedUrl) {
                // kind 12 = a piece-defined Custom event (§8.2's open channel): the front-end's
                // cx.on reads the text payload as the URL. (No longer hijacking kind 1 = TextChanged.)
                DayBridge.nativeOnEvent(id, 12, 0.0, finishedUrl);
            }

            @Override
            @SuppressWarnings("deprecation") // the String overload runs on every API level
            public boolean shouldOverrideUrlLoading(WebView view, String target) {
                if (!inline || target.startsWith(inlinePrefix)) {
                    return false; // in-site (or remote mode): let the WebView navigate
                }
                // Inline mode leaving the site: cancel and report (num -1 = the link report);
                // the Rust side runs the app's LinkPolicy — system browser by default.
                DayBridge.nativeOnEvent(id, 12, -1.0, target);
                return true;
            }
        });
        if (url != null && !url.isEmpty()) {
            web.loadUrl(url);
        }
        return web;
    }

    /** Imperative commands: 0=load, 1=back, 2=forward, 3=stop, 4=reload. */
    public static void webCommand(View view, int code, String url) {
        if (!(view instanceof WebView)) {
            return;
        }
        WebView web = (WebView) view;
        switch (code) {
            case 0:
                if (url != null && !url.isEmpty()) {
                    web.loadUrl(url);
                }
                break;
            case 1:
                if (web.canGoBack()) {
                    web.goBack();
                }
                break;
            case 2:
                if (web.canGoForward()) {
                    web.goForward();
                }
                break;
            case 3:
                web.stopLoading();
                break;
            case 4:
                web.reload();
                break;
            default:
                break;
        }
    }
}
