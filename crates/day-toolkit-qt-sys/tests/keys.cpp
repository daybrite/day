// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The full key route's contract at the native boundary (docs/menus.md "The full route"):
// what the key filter hands Day for real Qt events, text kept apart from keys, a ⌘ chord
// left to the menu, a claimed chord ahead of an application shortcut. Synthetic node IDs
// are test fixtures.
#include <QApplication>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QShortcut>
#include <QWidget>
#include <cassert>
#include <cstdint>
#include <string>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
void day_qt_enable_keys(void *, uint64_t, int (*)(uint64_t, int, int));
void day_qt_set_text_keys(int (*)(uint64_t), int (*)(uint64_t, const char *, const char *, int));
}

struct Received {
    std::string key;
    bool typed;
    std::string text;
    int mods;
};
static std::vector<Received> received;
static int query(uint64_t node) { return node == 42; }
static int key(uint64_t node, const char *name, const char *text, int mods) {
    assert(node == 42);
    received.push_back({name ? name : "", text != nullptr, text ? text : "", mods});
    return 1;
}
static int short_key(uint64_t, int, int) { return 1; }

// A press the way Qt delivers one: ShortcutOverride first (the shortcut map's question),
// then the KeyPress, unless a shortcut claimed it.
static void press(QWidget &w, int key, Qt::KeyboardModifiers m, const QString &text = QString(),
                  bool *shortcutWon = nullptr) {
    QKeyEvent so(QEvent::ShortcutOverride, key, m, text);
    QApplication::sendEvent(&w, &so);
    if (!so.isAccepted() && shortcutWon) {
        // Qt's shortcut map would run here; the test asks it directly.
        QKeyEvent kp(QEvent::KeyPress, key, m, text);
        *shortcutWon = !QApplication::sendEvent(&w, &kp);
        return;
    }
    QKeyEvent kp(QEvent::KeyPress, key, m, text);
    QApplication::sendEvent(&w, &kp);
}
static Received one() {
    assert(received.size() == 1);
    Received r = received.front();
    received.clear();
    return r;
}
static void none() {
    assert(received.empty());
}

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget w;
    day_qt_set_text_keys(query, key);
    day_qt_enable_keys(&w, 42, short_key);
    w.show();
    w.activateWindow();
    w.setFocus();
    QApplication::processEvents();
    // The input method addresses the widget once it took the keyboard and the focus.
    assert(w.testAttribute(Qt::WA_InputMethodEnabled));

    // A key that types: named by its character, carrying what it typed.
    press(w, Qt::Key_A, Qt::NoModifier, "a");
    { Received r = one(); assert(r.key == "a" && r.typed && r.text == "a" && r.mods == 0); }
    press(w, Qt::Key_A, Qt::ShiftModifier, "A");
    { Received r = one(); assert(r.key == "A" && r.typed && r.text == "A" && r.mods == 1); }
    // A key that types nothing: named, no text.
    press(w, Qt::Key_Return, Qt::NoModifier, "\r");
    { Received r = one(); assert(r.key == "Enter" && !r.typed); }
    press(w, Qt::Key_F5, Qt::NoModifier);
    { Received r = one(); assert(r.key == "F5" && !r.typed); }
    press(w, Qt::Key_Tab, Qt::NoModifier, "\t");
    { Received r = one(); assert(r.key == "Tab" && !r.typed); }

    // An input method's commit: text, with no key behind more than one character, and the
    // word "Enter" is text, never the key.
    QInputMethodEvent phrase;
    phrase.setCommitString(QString::fromUtf8("日本"));
    QApplication::sendEvent(&w, &phrase);
    { Received r = one(); assert(r.key.empty() && r.typed && r.text == "日本"); }
    QInputMethodEvent accent;
    accent.setCommitString(QString::fromUtf8("é"));
    QApplication::sendEvent(&w, &accent);
    { Received r = one(); assert(r.key == "é" && r.typed && r.text == "é"); }
    QInputMethodEvent word;
    word.setCommitString("Enter");
    QApplication::sendEvent(&w, &word);
    { Received r = one(); assert(r.key.empty() && r.typed && r.text == "Enter"); }
    // A composition in progress (preedit, no commit) types nothing yet.
    QInputMethodEvent preedit(QString::fromUtf8("に"), {});
    QApplication::sendEvent(&w, &preedit);
    none();

#ifdef Q_OS_MAC
    // ⌘ chords are the menu's, named key or not; ⌃ chords name their key and type nothing.
    press(w, Qt::Key_Left, Qt::ControlModifier);
    none();
    press(w, Qt::Key_C, Qt::ControlModifier, "c");
    none();
    press(w, Qt::Key_C, Qt::MetaModifier, QString(QChar(3)));
    { Received r = one(); assert(r.key == "c" && !r.typed && r.mods == 8); }
    press(w, Qt::Key_BracketLeft, Qt::MetaModifier, QString(QChar(27)));
    { Received r = one(); assert(r.key == "[" && !r.typed && r.mods == 8); }
    const Qt::KeyboardModifiers control = Qt::MetaModifier;
    const QKeySequence shortcutKeys(Qt::META | Qt::Key_C);
#else
    // Ctrl is the primary key and the Control key; a chord names its key and types nothing.
    press(w, Qt::Key_C, Qt::ControlModifier, QString(QChar(3)));
    { Received r = one(); assert(r.key == "c" && !r.typed && r.mods == (2 | 8)); }
    press(w, Qt::Key_BracketLeft, Qt::ControlModifier, QString(QChar(27)));
    { Received r = one(); assert(r.key == "[" && !r.typed && r.mods == (2 | 8)); }
    // AltGr (Ctrl+Alt to Qt on Windows) that typed a character is that text.
    press(w, Qt::Key_At, Qt::ControlModifier | Qt::AltModifier, "@");
    { Received r = one(); assert(r.key == "@" && r.typed && r.text == "@" && r.mods == 0); }
    const Qt::KeyboardModifiers control = Qt::ControlModifier;
    const QKeySequence shortcutKeys(Qt::CTRL | Qt::Key_C);
#endif
    press(w, Qt::Key_X, Qt::AltModifier);
    { Received r = one(); assert(r.key == "x" && !r.typed && (r.mods & 4)); }

    // A claimed chord wins over an application shortcut: the filter answers Qt's
    // ShortcutOverride, so the shortcut map never sees the press.
    int activations = 0;
    QShortcut shortcut(shortcutKeys, &w);
    QObject::connect(&shortcut, &QShortcut::activated, [&] { ++activations; });
    bool shortcutWon = false;
    press(w, Qt::Key_C, control, QString(QChar(3)), &shortcutWon);
    { Received r = one(); assert(r.key == "c" && !r.typed); }
    assert(!shortcutWon && activations == 0);

    // A widget that did not take the keyboard keeps the short route: a letter is nobody's.
    QWidget plain;
    day_qt_enable_keys(&plain, 7, short_key);
    plain.show();
    plain.setFocus();
    QApplication::processEvents();
    assert(!plain.testAttribute(Qt::WA_InputMethodEnabled));
    press(plain, Qt::Key_A, Qt::NoModifier, "a");
    none();
    return 0;
}
