// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The full key route's contract at the native boundary (docs/menus.md "The full route"):
// what the key filter hands Day for real Qt events, text kept apart from keys, a ⌘ chord
// left to the menu, a claimed chord ahead of an application shortcut. Synthetic node IDs
// are test fixtures.
#include <QApplication>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QLineEdit>
#include <QShortcut>
#include <QWidget>
#include <cassert>
#include <cstdint>
#include <string>
#include <vector>

extern "C" {
void day_qt_open_file(const char *) {}
void day_qt_enable_keys(void *, uint64_t, int (*)(uint64_t, int, int, int));
void day_qt_set_text_keys(int (*)(uint64_t), int (*)(uint64_t, const char *, const char *, int, int));
void day_qt_lineedit_enable_field_keys(void *, uint64_t, int (*)(uint64_t, const char *, int, int, int));
void day_qt_lineedit_select(void *, int, int);
}

struct Received {
    std::string key;
    bool typed;
    std::string text;
    int mods;
    int phase;
};
static std::vector<Received> received;
static int query(uint64_t node) { return node == 42; }
static int key(uint64_t node, const char *name, const char *text, int mods, int phase) {
    assert(node == 42);
    received.push_back({name ? name : "", text != nullptr, text ? text : "", mods, phase});
    return 1;
}
// The short route's reports: (code, phase).
static std::vector<std::pair<int, int>> short_keys;
static int short_key(uint64_t, int code, int, int phase) {
    short_keys.push_back({code, phase});
    return 1;
}
// A line edit's claimed keys (docs/textfield.md "Keys"): the plain ones, as Day's
// `keys::field_claims` answers for a field with a handler.
static std::vector<std::string> field_keys;
static int field_key(uint64_t node, const char *name, int mods, int deliver, int phase) {
    assert(node == 9);
    if (phase == 2) {
        field_keys.push_back(std::string("up ") + name);
        return 1;
    }
    if (mods != 0) return 0;
    if (deliver) field_keys.push_back(phase == 1 ? std::string("repeat ") + name : std::string(name));
    return 1;
}
// A release the way Qt delivers one; `repeat` marks the release X11 sends between repeats.
static void release(QWidget &w, int key, Qt::KeyboardModifiers m, const QString &text = QString(),
                    bool repeat = false) {
    QKeyEvent kr(QEvent::KeyRelease, key, m, text, repeat);
    QApplication::sendEvent(&w, &kr);
}
static void repeatPress(QWidget &w, int key, Qt::KeyboardModifiers m, const QString &text = QString()) {
    QKeyEvent kp(QEvent::KeyPress, key, m, text, true);
    QApplication::sendEvent(&w, &kp);
}

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
    { Received r = one(); assert(r.key == "a" && r.typed && r.text == "a" && r.mods == 0 && r.phase == 0); }
    // Held, it repeats with its text (X11 sends a release between repeats: dropped), and its
    // release is the same key, with no text, and is claimed.
    release(w, Qt::Key_A, Qt::NoModifier, "a", true);
    none();
    repeatPress(w, Qt::Key_A, Qt::NoModifier, "a");
    { Received r = one(); assert(r.key == "a" && r.typed && r.text == "a" && r.phase == 1); }
    {
        QKeyEvent up(QEvent::KeyRelease, Qt::Key_A, Qt::NoModifier, "a");
        const bool claimed = QApplication::sendEvent(&w, &up);
        assert(claimed);
        Received r = one();
        assert(r.key == "a" && !r.typed && r.phase == 2);
    }
    // A key released twice (or never pressed while focused) has no second up.
    release(w, Qt::Key_A, Qt::NoModifier, "a");
    none();
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
    assert(short_keys.empty());
    // The short route's phases: down, repeat, the up of a claimed down; no up after the focus
    // left (the key went down while focused and came up elsewhere).
    press(plain, Qt::Key_Right, Qt::NoModifier);
    repeatPress(plain, Qt::Key_Right, Qt::NoModifier);
    release(plain, Qt::Key_Right, Qt::NoModifier, QString(), true);
    release(plain, Qt::Key_Right, Qt::NoModifier);
    assert((short_keys == std::vector<std::pair<int, int>>{{1, 0}, {1, 1}, {1, 2}}));
    short_keys.clear();
    press(plain, Qt::Key_Left, Qt::NoModifier);
    QFocusEvent out(QEvent::FocusOut, Qt::TabFocusReason);
    QApplication::sendEvent(&plain, &out);
    release(plain, Qt::Key_Left, Qt::NoModifier);
    assert((short_keys == std::vector<std::pair<int, int>>{{0, 0}}));
    short_keys.clear();

    // A line edit with a key handler: plain Up, Down and Escape are the app's, ahead of the
    // field and of an Escape shortcut (a sheet's back); a modified arrow stays the field's.
    QLineEdit field(QString::fromUtf8("a😀b.example"));
    day_qt_lineedit_enable_field_keys(&field, 9, field_key);
    field.show();
    field.activateWindow();
    field.setFocus();
    QApplication::processEvents();
    field.setCursorPosition(4);
    int escapes = 0;
    QShortcut escape(QKeySequence(Qt::Key_Escape), &field);
    QObject::connect(&escape, &QShortcut::activated, [&] { ++escapes; });
    bool escapeWon = false;
    press(field, Qt::Key_Escape, Qt::NoModifier, QString(QChar(27)), &escapeWon);
    assert(!escapeWon && escapes == 0);
    press(field, Qt::Key_Down, Qt::NoModifier);
    repeatPress(field, Qt::Key_Down, Qt::NoModifier);
    release(field, Qt::Key_Down, Qt::NoModifier);
    press(field, Qt::Key_Up, Qt::NoModifier);
    press(field, Qt::Key_Up, Qt::ShiftModifier);
    assert((field_keys == std::vector<std::string>{"Escape", "ArrowDown", "repeat ArrowDown",
                                                   "up ArrowDown", "ArrowUp"}));
    assert(field.cursorPosition() == 4);
    // Selection in characters: the emoji is one character and two UTF-16 units.
    day_qt_lineedit_select(&field, 2, 3);
    assert(field.selectionStart() == 3 && field.selectedText() == "b");
    assert(field.cursorPosition() == 4);
    day_qt_lineedit_select(&field, 3, 1000);
    assert(field.selectedText() == ".example");
    day_qt_lineedit_select(&field, 0, 1000);
    assert(field.selectedText() == field.text());
    day_qt_lineedit_select(&field, 5, 5);
    assert(!field.hasSelectedText() && field.cursorPosition() == 6);
    return 0;
}
