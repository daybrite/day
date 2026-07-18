// The datetime piece's own Qt shim (the day-piece-picker recipe): date and time controls behind a
// flat C ABI. Compact date = QDateEdit with a QCalendarWidget popup (the Qt idiom); inline date =
// QCalendarWidget; time = QTimeEdit (Qt has no inline clock — the sectioned field IS its time UI).
// Values cross as epoch days / seconds-of-day (int64). Programmatic sets run under blockSignals so
// they never echo back.

#include <QCalendarWidget>
#include <QDateEdit>
#include <QTimeEdit>
#include <QVBoxLayout>
#include <QWidget>

#include <cstdint>

namespace {

QDate fromEpochDays(int64_t days) {
    // Julian day 2440588 == 1970-01-01.
    return QDate::fromJulianDay(days + 2440588);
}

int64_t toEpochDays(const QDate &d) { return d.toJulianDay() - 2440588; }

class DayDateWidget : public QWidget {
public:
    QDateEdit *edit = nullptr;
    QCalendarWidget *calendar = nullptr;
    void set(int64_t days) {
        QDate d = fromEpochDays(days);
        if (edit && edit->date() != d) {
            edit->blockSignals(true);
            edit->setDate(d);
            edit->blockSignals(false);
        } else if (calendar && calendar->selectedDate() != d) {
            calendar->blockSignals(true);
            calendar->setSelectedDate(d);
            calendar->blockSignals(false);
        }
    }
};

class DayTimeWidget : public QWidget {
public:
    QTimeEdit *edit = nullptr;
    void set(int64_t secs) {
        QTime t = QTime(0, 0).addSecs(static_cast<int>(secs));
        if (edit && edit->time() != t) {
            edit->blockSignals(true);
            edit->setTime(t);
            edit->blockSignals(false);
        }
    }
};

} // namespace

extern "C" {

void *day_datetime_date_new(int inline_style, int64_t epoch_days, int has_min, int64_t min_days,
                            int has_max, int64_t max_days, uint64_t id,
                            void (*cb)(uint64_t, int64_t)) {
    DayDateWidget *w = new DayDateWidget();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QDate value = fromEpochDays(epoch_days);
    if (inline_style) {
        QCalendarWidget *c = new QCalendarWidget();
        c->setSelectedDate(value);
        if (has_min)
            c->setMinimumDate(fromEpochDays(min_days));
        if (has_max)
            c->setMaximumDate(fromEpochDays(max_days));
        QObject::connect(c, &QCalendarWidget::selectionChanged,
                         [c, id, cb]() { cb(id, toEpochDays(c->selectedDate())); });
        lay->addWidget(c);
        w->calendar = c;
    } else {
        QDateEdit *e = new QDateEdit(value);
        e->setCalendarPopup(true); // field → calendar popup, the Qt compact idiom
        if (has_min)
            e->setMinimumDate(fromEpochDays(min_days));
        if (has_max)
            e->setMaximumDate(fromEpochDays(max_days));
        QObject::connect(e, &QDateEdit::dateChanged,
                         [id, cb](QDate d) { cb(id, toEpochDays(d)); });
        lay->addWidget(e);
        w->edit = e;
    }
    return w;
}

void day_datetime_date_set(void *w, int64_t epoch_days) {
    static_cast<DayDateWidget *>(w)->set(epoch_days);
}

void *day_datetime_time_new(int with_seconds, int64_t secs, uint64_t id,
                            void (*cb)(uint64_t, int64_t)) {
    DayTimeWidget *w = new DayTimeWidget();
    QVBoxLayout *lay = new QVBoxLayout(w);
    lay->setContentsMargins(0, 0, 0, 0);
    QTimeEdit *e = new QTimeEdit(QTime(0, 0).addSecs(static_cast<int>(secs)));
    if (with_seconds)
        e->setDisplayFormat(QStringLiteral("HH:mm:ss")); // locale default has no seconds field
    QObject::connect(e, &QTimeEdit::timeChanged,
                     [id, cb](QTime t) { cb(id, QTime(0, 0).secsTo(t)); });
    lay->addWidget(e);
    w->edit = e;
    return w;
}

void day_datetime_time_set(void *w, int64_t secs) { static_cast<DayTimeWidget *>(w)->set(secs); }

} // extern "C"
