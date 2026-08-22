// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-stepper's OWN Qt shim: a `QDoubleSpinBox` behind a flat C ABI — Qt's
// field-with-arrows widget, range/step/decimals and keyboard entry included. The value
// crosses as a double both ways; the suppress flag keeps day's own writes from echoing back
// through `valueChanged` as user steps.

#include <QDoubleSpinBox>

#include <cstdint>

namespace {

class DayStepper : public QDoubleSpinBox {
public:
    bool suppress = false;
};

} // namespace

extern "C" {

void *day_stepper_new(double value, double min, double max, double step, int decimals,
                      uint64_t id, void (*cb)(uint64_t, double)) {
    DayStepper *w = new DayStepper();
    w->setRange(min, max);
    w->setSingleStep(step);
    w->setDecimals(decimals);
    w->suppress = true;
    w->setValue(value);
    w->suppress = false;
    QObject::connect(w, QOverload<double>::of(&QDoubleSpinBox::valueChanged),
                     [w, id, cb](double v) {
                         if (!w->suppress)
                             cb(id, v);
                     });
    return w;
}

void day_stepper_set(void *h, double value) {
    DayStepper *w = static_cast<DayStepper *>(h);
    w->suppress = true;
    w->setValue(value);
    w->suppress = false;
}

} // extern "C"
