// day-piece-datetime's OWN ArkUI shim (docs/extending.md, the pullrefresh recipe): the NDK picker
// nodes, API 12+ — ARKUI_NODE_CALENDAR_PICKER (compact date: entry field → calendar popup),
// ARKUI_NODE_DATE_PICKER (inline date wheels; carries native START/END bounds), and
// ARKUI_NODE_TIME_PICKER (time wheels). The shim queries its OWN ArkUI_NativeNodeAPI_1 and uses
// the ADDITIVE per-node event receiver (addNodeEventReceiver) so it never touches
// day-arkui-sys's global receiver; each node's event user data carries the day NodeId.
//
// Payload notes straight from native_node.h: the wheels DATE_PICKER reports month 0–11 (the
// calendar picker and the SELECTED_DATE attribute use 1–12) — normalized to 1–12 at this
// boundary; the wheels SELECTED/START/END attributes are "YYYY-M-D" strings. A null createNode
// (picker nodes missing from this SDK) returns nullptr — day falls back per docs.

#include <cstdint>
#include <cstdio>
#include <unordered_map>

#include <arkui/native_interface.h>
#include <arkui/native_node.h>
#include <arkui/native_type.h>

static ArkUI_NativeNodeAPI_1* dtp_api() {
    static ArkUI_NativeNodeAPI_1* api = nullptr;
    if (!api) {
        api = reinterpret_cast<ArkUI_NativeNodeAPI_1*>(
            OH_ArkUI_QueryModuleInterfaceByName(ARKUI_NATIVE_NODE, "ArkUI_NativeNodeAPI_1"));
    }
    return api;
}

// One callback per addon (set at first node creation) — the Rust side routes by NodeId.
static void (*g_on_date)(long long, int, int, int) = nullptr; // id, year, month(1-12), day
static void (*g_on_time)(long long, int, int) = nullptr;      // id, hour, minute

// Which date nodes are the wheels DATE_PICKER (value patches use the string attribute) vs the
// CALENDAR_PICKER (u32-triple attribute).
static std::unordered_map<void*, bool>& wheels_map() {
    static std::unordered_map<void*, bool> m;
    return m;
}

static void dtp_event(ArkUI_NodeEvent* ev) {
    ArkUI_NodeComponentEvent* ce = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
    if (!ce) return;
    long long id = (long long)(uintptr_t)OH_ArkUI_NodeEvent_GetUserData(ev);
    switch (OH_ArkUI_NodeEvent_GetEventType(ev)) {
    case NODE_CALENDAR_PICKER_EVENT_ON_CHANGE: // u32 year / month (1-12) / day
        if (g_on_date)
            g_on_date(id, (int)ce->data[0].u32, (int)ce->data[1].u32, (int)ce->data[2].u32);
        break;
    case NODE_DATE_PICKER_EVENT_ON_DATE_CHANGE: // i32 year / month (0-11!) / day
        if (g_on_date)
            g_on_date(id, ce->data[0].i32, ce->data[1].i32 + 1, ce->data[2].i32);
        break;
    case NODE_TIME_PICKER_EVENT_ON_CHANGE: // i32 hour / minute
        if (g_on_time) g_on_time(id, ce->data[0].i32, ce->data[1].i32);
        break;
    default:
        break;
    }
}

static void set_calendar_date(ArkUI_NodeHandle n, int y, int m, int d) {
    ArkUI_NativeNodeAPI_1* api = dtp_api();
    ArkUI_NumberValue v[3];
    v[0].u32 = (uint32_t)y;
    v[1].u32 = (uint32_t)m;
    v[2].u32 = (uint32_t)d;
    ArkUI_AttributeItem item = {v, 3, nullptr, nullptr};
    api->setAttribute(n, NODE_CALENDAR_PICKER_SELECTED_DATE, &item);
}

static void set_wheels_date(ArkUI_NodeHandle n, int y, int m, int d) {
    ArkUI_NativeNodeAPI_1* api = dtp_api();
    char buf[16];
    snprintf(buf, sizeof buf, "%d-%d-%d", y, m, d);
    ArkUI_AttributeItem item = {nullptr, 0, buf, nullptr};
    api->setAttribute(n, NODE_DATE_PICKER_SELECTED, &item);
}

extern "C" void* day_dtp_date_new(long long id, int inline_style, int y, int m, int d,
                                  const char* min_iso, const char* max_iso,
                                  void (*cb)(long long, int, int, int)) {
    g_on_date = cb;
    ArkUI_NativeNodeAPI_1* api = dtp_api();
    if (!api) return nullptr;
    if (inline_style) {
        ArkUI_NodeHandle n = api->createNode(ARKUI_NODE_DATE_PICKER);
        if (!n) return nullptr; // unavailable on this SDK — day falls back per docs
        // Native bounds (the wheels won't scroll outside them); "" = leave the node default.
        if (min_iso && *min_iso) {
            ArkUI_AttributeItem item = {nullptr, 0, min_iso, nullptr};
            api->setAttribute(n, NODE_DATE_PICKER_START, &item);
        }
        if (max_iso && *max_iso) {
            ArkUI_AttributeItem item = {nullptr, 0, max_iso, nullptr};
            api->setAttribute(n, NODE_DATE_PICKER_END, &item);
        }
        set_wheels_date(n, y, m, d);
        api->registerNodeEvent(n, NODE_DATE_PICKER_EVENT_ON_DATE_CHANGE, 0, (void*)(uintptr_t)id);
        api->addNodeEventReceiver(n, dtp_event);
        wheels_map()[n] = true;
        return n;
    }
    // Compact: the calendar-picker entry field (no min/max attribute in the NDK — the piece's own
    // clamp bounds the VALUE; docs/datepicker.md).
    ArkUI_NodeHandle n = api->createNode(ARKUI_NODE_CALENDAR_PICKER);
    if (!n) return nullptr;
    set_calendar_date(n, y, m, d);
    api->registerNodeEvent(n, NODE_CALENDAR_PICKER_EVENT_ON_CHANGE, 0, (void*)(uintptr_t)id);
    api->addNodeEventReceiver(n, dtp_event);
    wheels_map()[n] = false;
    return n;
}

extern "C" void day_dtp_date_set(void* node, int y, int m, int d) {
    if (!node || !dtp_api()) return;
    auto it = wheels_map().find(node);
    if (it != wheels_map().end() && it->second)
        set_wheels_date(reinterpret_cast<ArkUI_NodeHandle>(node), y, m, d);
    else
        set_calendar_date(reinterpret_cast<ArkUI_NodeHandle>(node), y, m, d);
}

extern "C" void* day_dtp_time_new(long long id, int hour, int minute,
                                  void (*cb)(long long, int, int)) {
    g_on_time = cb;
    ArkUI_NativeNodeAPI_1* api = dtp_api();
    if (!api) return nullptr;
    ArkUI_NodeHandle n = api->createNode(ARKUI_NODE_TIME_PICKER);
    if (!n) return nullptr;
    char buf[8];
    snprintf(buf, sizeof buf, "%02d:%02d", hour, minute);
    ArkUI_AttributeItem item = {nullptr, 0, buf, nullptr};
    api->setAttribute(n, NODE_TIME_PICKER_SELECTED, &item);
    api->registerNodeEvent(n, NODE_TIME_PICKER_EVENT_ON_CHANGE, 0, (void*)(uintptr_t)id);
    api->addNodeEventReceiver(n, dtp_event);
    return n;
}

extern "C" void day_dtp_time_set(void* node, int hour, int minute) {
    if (!node || !dtp_api()) return;
    char buf[8];
    snprintf(buf, sizeof buf, "%02d:%02d", hour, minute);
    ArkUI_AttributeItem item = {nullptr, 0, buf, nullptr};
    dtp_api()->setAttribute(reinterpret_cast<ArkUI_NodeHandle>(node), NODE_TIME_PICKER_SELECTED,
                            &item);
}
