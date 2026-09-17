// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include <QApplication>
#include <QDrag>
#include <QMimeData>
#include <QMouseEvent>
#include <QDropEvent>
#include <QDragEnterEvent>
#include <QWidget>
#include <QPointer>
#include <QUrl>
#include <QDataStream>
#ifdef Q_OS_MACOS
#include <QUtiMimeConverter>
#include <CoreServices/CoreServices.h>
// GTK uses system MIME/UTI mappings; Qt's default custom MIME wrapper is Qt-specific.
// Register a converter so arbitrary custom data also has a shared native representation.
class DayUtiConverter : public QUtiMimeConverter {
    static QString string(CFStringRef s) {
        if (!s) return {};
        char bytes[4096]; bool ok = CFStringGetCString(s,bytes,sizeof(bytes),kCFStringEncodingUTF8);
        CFRelease(s); return ok ? QString::fromUtf8(bytes) : QString();
    }
public:
    QString utiForMime(const QString &mime) const override {
        if (!mime.startsWith("application/")) return {};
        auto utf = mime.toUtf8();
        auto tag = CFStringCreateWithCString(nullptr,utf.constData(),kCFStringEncodingUTF8);
        auto uti = UTTypeCreatePreferredIdentifierForTag(kUTTagClassMIMEType,tag,nullptr);
        CFRelease(tag); return string(uti);
    }
    QString mimeForUti(const QString &uti) const override {
        auto utf = uti.toUtf8(); auto tag = CFStringCreateWithCString(nullptr,utf.constData(),kCFStringEncodingUTF8);
        auto mime = UTTypeCopyPreferredTagWithClass(tag,kUTTagClassMIMEType); CFRelease(tag);
        auto result = string(mime); return result.startsWith("application/") ? result : QString();
    }
    QList<QByteArray> convertFromMime(const QString &,const QVariant &data,const QString &) const override { return {data.toByteArray()}; }
    QVariant convertToMime(const QString &,const QList<QByteArray> &data,const QString &) const override { return data.isEmpty() ? QVariant() : QVariant(data.first()); }
};
#endif
static constexpr auto bundle = "application/vnd.day.transfer";
static constexpr qsizetype limit = 64 * 1024 * 1024;
using Prepare = unsigned char* (*)(void*,double,double,size_t*);
using Free = void (*)(unsigned char*,size_t);
using Accept = bool (*)(void*,double,double,const char*,bool);
using Receive = bool (*)(void*,double,double,const unsigned char*,size_t,bool);

static QMimeData* dataFromPacket(const QByteArray &packet) {
    auto mime = new QMimeData;
    mime->setData(bundle, packet);
    QDataStream in(packet); in.setByteOrder(QDataStream::LittleEndian);
    in.skipRawData(8); quint32 items, count; in >> items >> count;
    // Standard alternatives for the first item; the bundle retains all item boundaries.
    for (quint32 i=0; i<count && i<32 && in.status()==QDataStream::Ok; ++i) {
        quint32 m,n; in >> m >> n;
        if(m>255 || n>limit) break;
        QByteArray type(m,0),bytes(n,0); in.readRawData(type.data(),m); in.readRawData(bytes.data(),n);
        if(type=="text/uri-list") {
            QList<QUrl> urls; for (auto line:bytes.split('\n')) if(!line.trimmed().isEmpty() && !line.startsWith('#')) urls.append(QUrl::fromEncoded(line.trimmed()));
            mime->setUrls(urls);
        } else { mime->setData(QString::fromUtf8(type),bytes); }
    }
    return mime;
}
static QByteArray packetFromData(const QMimeData *mime) {
    if(mime->hasFormat(bundle)) return mime->data(bundle);
    QList<QPair<QByteArray,QByteArray>> reps;
    qsizetype total=0;
    for(auto type:mime->formats()) {
        auto name=type.toUtf8(); if(name.size()>255 || name.contains(';')) continue;
        auto bytes=mime->data(type); total+=bytes.size(); if(total>limit || reps.size()>=32) return {};
        reps.append({name,bytes});
    }
    QByteArray result("DAYDND\0\1",8); QDataStream out(&result,QIODevice::Append); out.setByteOrder(QDataStream::LittleEndian);
    out << quint32(1) << quint32(reps.size());
    for(auto r:reps) { out << quint32(r.first.size()) << quint32(r.second.size()); out.writeRawData(r.first.data(),r.first.size()); out.writeRawData(r.second.data(),r.second.size()); }
    return result;
}
class DayTransfer : public QObject {
    QPointer<QWidget> view;
    QPoint start;
    bool armed=false;
public:
    Prepare prepare=nullptr; Free release=nullptr; Accept accept=nullptr; Receive receive=nullptr;
    explicit DayTransfer(QWidget *v):QObject(v),view(v) { qApp->installEventFilter(this); v->setProperty("dayTransfer",QVariant::fromValue((void*)this)); }
    bool eventFilter(QObject *object,QEvent *event) override {
        auto w=qobject_cast<QWidget*>(object); if(!view || !w) return false;
        if(prepare && (event->type()==QEvent::MouseButtonPress || event->type()==QEvent::MouseMove || event->type()==QEvent::MouseButtonRelease)) {
            QWidget *owner=w; while(owner && !owner->property("dayTransferSource").toBool()) owner=owner->parentWidget();
            if(owner==view) {
                auto e=static_cast<QMouseEvent*>(event);
                if(event->type()==QEvent::MouseButtonPress && e->button()==Qt::LeftButton) { start=e->globalPosition().toPoint(); armed=true; }
                if(event->type()==QEvent::MouseButtonRelease) armed=false;
                if(event->type()==QEvent::MouseMove && armed && (e->buttons() & Qt::LeftButton) && (e->globalPosition().toPoint()-start).manhattanLength()>=QApplication::startDragDistance()) {
                    armed=false; auto at=view->mapFromGlobal(start); size_t n=0; auto p=prepare(view,at.x(),at.y(),&n);
                    if(!p) return false;
                    QByteArray packet((const char*)p,n); release(p,n);
                    QPointer<QDrag> drag=new QDrag(view); drag->setMimeData(dataFromPacket(packet)); drag->setPixmap(view->grab()); drag->setHotSpot(at);
                    drag->exec(Qt::CopyAction,Qt::CopyAction); if (drag) drag->deleteLater(); return true;
                }
            }
        }
        if(w!=view || !accept) return false;
        if(event->type()==QEvent::DragEnter || event->type()==QEvent::DragMove || event->type()==QEvent::Drop) {
            auto e=static_cast<QDropEvent*>(event); auto types=e->mimeData()->formats().join('\n').toUtf8();
            bool ok=(e->possibleActions() & Qt::CopyAction) && accept(view,e->position().x(),e->position().y(),types.constData(),e->source()!=nullptr);
            if(ok && event->type()==QEvent::Drop) { auto packet=packetFromData(e->mimeData()); ok=packet.size()<=limit && receive(view,e->position().x(),e->position().y(),(const unsigned char*)packet.constData(),packet.size(),e->source()!=nullptr); }
            if(ok) { e->setDropAction(Qt::CopyAction); e->accept(); } else e->ignore();
            return true;
        }
        return false;
    }
};
static DayTransfer* controller(QWidget *w) {
#ifdef Q_OS_MACOS
    static DayUtiConverter converter;
#endif
    auto c=(DayTransfer*)w->property("dayTransfer").value<void*>(); return c ? c : new DayTransfer(w);
}
extern "C" void day_qt_drag_source(void *w,Prepare p,Free f) { auto v=(QWidget*)w; auto c=controller(v); c->prepare=p; c->release=f; v->setProperty("dayTransferSource",true); }
extern "C" void day_qt_drop_target(void *w,Accept a,Receive r) { auto v=(QWidget*)w; auto c=controller(v); c->accept=a; c->receive=r; v->setAcceptDrops(true); }
