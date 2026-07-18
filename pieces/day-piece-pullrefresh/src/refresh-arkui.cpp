// day-piece-pullrefresh's OWN ArkUI shim (docs/extending.md): create an ARKUI_NODE_REFRESH (API
// 12+) that Day mounts the wrapped scrollable into (the generic day_ark_insert_child accepts it —
// Refresh hosts exactly one child). Pull-begins arrive via NODE_REFRESH_ON_REFRESH; the indicator
// is driven both ways through NODE_REFRESH_REFRESHING. The shim queries its OWN
// ArkUI_NativeNodeAPI_1 and uses the ADDITIVE per-node event receiver (addNodeEventReceiver), so
// it never touches day-arkui-sys's global receiver; the per-node event's user data carries the day
// NodeId, matching the sys shim's convention should the global receiver also observe the event
// (unknown event types fall through its switch harmlessly).

#include <cstdint>

#include <arkui/native_interface.h>
#include <arkui/native_node.h>
#include <arkui/native_type.h>

static ArkUI_NativeNodeAPI_1* prf_api() {
    static ArkUI_NativeNodeAPI_1* api = nullptr;
    if (!api) {
        api = reinterpret_cast<ArkUI_NativeNodeAPI_1*>(
            OH_ArkUI_QueryModuleInterfaceByName(ARKUI_NATIVE_NODE, "ArkUI_NativeNodeAPI_1"));
    }
    return api;
}

// One callback per addon (set at first node creation) — the Rust side routes by NodeId.
static void (*g_on_refresh)(long long) = nullptr;

static void prf_event(ArkUI_NodeEvent* ev) {
    if (OH_ArkUI_NodeEvent_GetEventType(ev) == NODE_REFRESH_ON_REFRESH && g_on_refresh) {
        long long id = (long long)(uintptr_t)OH_ArkUI_NodeEvent_GetUserData(ev);
        g_on_refresh(id);
    }
}

extern "C" void day_prf_set_refreshing(void* node, int on) {
    ArkUI_NativeNodeAPI_1* api = prf_api();
    if (!api || !node) return;
    ArkUI_NumberValue v;
    v.i32 = on ? 1 : 0;
    ArkUI_AttributeItem item = {&v, 1, nullptr, nullptr};
    api->setAttribute(reinterpret_cast<ArkUI_NodeHandle>(node), NODE_REFRESH_REFRESHING, &item);
}

extern "C" void* day_prf_node_new(long long id, int refreshing, void (*cb)(long long)) {
    g_on_refresh = cb;
    ArkUI_NativeNodeAPI_1* api = prf_api();
    if (!api) return nullptr;
    ArkUI_NodeHandle n = api->createNode(ARKUI_NODE_REFRESH);
    if (!n) return nullptr; // Refresh unavailable on this SDK — day falls back per docs
    api->registerNodeEvent(n, NODE_REFRESH_ON_REFRESH, 0, (void*)(uintptr_t)id);
    api->addNodeEventReceiver(n, prf_event);
    if (refreshing) day_prf_set_refreshing(n, 1);
    return n;
}
