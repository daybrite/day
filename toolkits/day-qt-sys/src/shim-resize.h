// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#pragma once

#include <QEvent>
#include <QObject>
#include <QPointer>
#include <QTimer>
#include <QWidget>

// QWidget::render (including QGraphicsEffect::sourcePixmap) delivers pending resize
// events inside a paint. Calling Day there can drain unrelated queued UI actions,
// reparent native WebViews, and destroy the backing store that Qt is still painting.
// Report the latest geometry after the native event has returned. The filter owns
// the scheduled callback; deleting the pane cancels it, and deleting the host is safe.
class DayPaneResizeFilter : public QObject {
    QPointer<QWidget> host;
    void (*cb)(void *);
    bool pending = false;

public:
    DayPaneResizeFilter(QWidget *h, void (*c)(void *)) : host(h), cb(c) {}

protected:
    bool eventFilter(QObject *obj, QEvent *ev) override {
        if (ev->type() == QEvent::Resize && !pending) {
            pending = true;
            QTimer::singleShot(0, this, [this] {
                pending = false;
                if (host) cb(host.data());
                // The callback may dispose this filter. Do not access it again.
            });
        }
        return QObject::eventFilter(obj, ev);
    }
};
