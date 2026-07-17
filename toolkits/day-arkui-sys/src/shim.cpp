// day-arkui-sys — a flat C ABI over the HarmonyOS ArkUI Native NodeAPI (arkui/native_node.h) and
// NAPI (napi/native_api.h), the HarmonyOS analogue of day-qt-sys / day-winui-sys. day builds the
// widget tree natively (createNode/setAttribute/addChild) and mounts it into an ArkTS `NodeContent`
// slot; native events call back into Rust by node id; main-thread posting rides libuv (uv_async).
//
// The ArkUI headers assume C++ (bool, forward-declared types), so this is compiled as C++.

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <map>
#include <mutex>
#include <string>

#include <sys/mman.h>
#include <unistd.h>

#include <arkui/native_interface.h>
#include <arkui/native_interface_focus.h>
#include <arkui/native_node.h>
#include <arkui/native_node_napi.h>
#include <arkui/native_type.h>
#include <hilog/log.h>
#include <napi/native_api.h>
#include <rawfile/raw_file.h>
#include <rawfile/raw_file_manager.h>
#include <uv.h>

// OH_Drawing (native 2-D) for the canvas custom node's on-draw callback (§11).
#include <native_drawing/drawing_brush.h>
#include <native_drawing/drawing_canvas.h>
#include <native_drawing/drawing_font.h>
#include <native_drawing/drawing_matrix.h>
#include <native_drawing/drawing_path.h>
#include <native_drawing/drawing_pen.h>
#include <native_drawing/drawing_point.h>
#include <native_drawing/drawing_rect.h>
#include <native_drawing/drawing_round_rect.h>
#include <native_drawing/drawing_shader_effect.h>
#include <native_drawing/drawing_text_blob.h>

#include <map>
#include <vector>

// ---- globals ---------------------------------------------------------------
static ArkUI_NativeNodeAPI_1* g_api = nullptr;
static napi_env g_env = nullptr;
static double g_density = 1.0; // px per vp; ArkUI attributes are vp, measure/layout are px

// The app's native resource manager (§18.3), captured from the ArkTS `resourceManager` via the
// `registerResourceManager` NAPI export. Needed to read staged rawfile data resources; null until
// the entry ability registers it (in which case the rawfile opener returns nothing).
static NativeResourceManager* g_res_mgr = nullptr;

// Implemented in Rust (the day-arkui backend / the app cdylib).
extern "C" void day_arkui_start(void* content, double w_vp, double h_vp, double density);
extern "C" void day_arkui_on_event(uint64_t id, int32_t kind, double num, const char* text);
extern "C" void day_arkui_set_cache_dir(const char* path);
// Recycling-list callbacks into Rust (docs/list.md): row count, and build/rebind a row's content
// into the native cell (a plain Stack `cell`) — plus recycle when a cell scrolls out.
extern "C" uint32_t day_arkui_list_count(uint64_t host_id);
extern "C" void day_arkui_list_bind(uint64_t host_id, uint32_t index, void* cell);

// The ArkTS-registered file-picker callback (docs/files.md): `(req, mode, name, src, filters)`.
// Held as a napi_ref because HarmonyOS file pickers live in the ArkTS @kit.CoreFileKit layer,
// not the native NodeAPI. Called on the JS thread (day's loop runs there), so no threadsafe fn.
static napi_ref g_file_picker = nullptr;

// Opening a URL needs the UIAbility context's startAbility (a viewData Want), which lives in the
// ArkTS layer — the native NodeAPI has no equivalent. ArkTS registers `registerOpenUrl(cb)` where
// `cb` is `(url: string) => void`; day_ark_open_url invokes it. Null (unregistered) is a safe no-op.
static napi_ref g_open_url = nullptr;

// ---- Navigation bridge (docs/navigation.md) ---------------------------------
// Day drives the ArkTS `Navigation` / `NavPathStack` — HarmonyOS's own navigation system — the
// way it drives androidx fragments on Android: ArkTS registers push/pop/title callbacks
// (`registerNav`), each pushed Day page is mounted into a fresh ArkTS `NodeContent` rendered
// inside a `NavDestination` (system back gesture, title bar, transitions all native), and the
// ArkTS side reports destination disappearance (`navPopped`) + content size (`navPageArea`).
static napi_ref g_nav_push = nullptr;  // (key: number, title: string) => NodeContent
static napi_ref g_nav_pop = nullptr;   // () => void — pathStack.pop()
static napi_ref g_nav_title = nullptr; // (title: string) => void — retitle the top destination
// A pushed page's slot: the NodeContent handle PLUS a strong napi_ref on the JS object. The
// ArkTS side drops its own reference when the NavDestination disappears (onDisAppear), so
// without the ref the content is GC'd while Rust may still detach the page from it — the
// RemoveNode-after-pop then walks freed FrameNodes (SIGSEGV in ViewModel::RemoveChild).
struct DayNavContent {
    ArkUI_NodeContentHandle content;
    napi_ref ref;
};
static std::map<uint64_t, DayNavContent> g_nav_contents;
extern "C" void day_arkui_nav_popped(uint64_t key);
extern "C" void day_arkui_nav_area(uint64_t key, double w, double h);

// ---- main-thread posting (uv_async on the JS event loop) -------------------
struct PostItem {
    void (*cb)(void*);
    void* data;
};
static uv_async_t g_async;
static std::mutex g_mtx;
static std::deque<PostItem> g_queue;
static bool g_async_ready = false;

static void drain_async(uv_async_t*) {
    for (;;) {
        PostItem it;
        {
            std::lock_guard<std::mutex> lk(g_mtx);
            if (g_queue.empty()) break;
            it = g_queue.front();
            g_queue.pop_front();
        }
        it.cb(it.data);
    }
}

// ---- native event receiver → Rust ------------------------------------------
static void event_receiver(ArkUI_NodeEvent* ev) {
    if (!ev) return;
    uint64_t id = (uint64_t)(uintptr_t)OH_ArkUI_NodeEvent_GetUserData(ev);
    ArkUI_NodeEventType t = OH_ArkUI_NodeEvent_GetEventType(ev);
    switch (t) {
        case NODE_ON_CLICK:
            day_arkui_on_event(id, 0, 0.0, "");
            break;
        case NODE_TEXT_INPUT_ON_CHANGE: {
            auto* s = OH_ArkUI_NodeEvent_GetStringAsyncEvent(ev);
            day_arkui_on_event(id, 1, 0.0, (s && s->pStr) ? s->pStr : "");
            break;
        }
        case NODE_TOGGLE_ON_CHANGE: {
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, 2, c ? (double)c->data[0].i32 : 0.0, "");
            break;
        }
        case NODE_SLIDER_EVENT_ON_CHANGE: {
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, 3, c ? (double)c->data[0].f32 : 0.0, "");
            break;
        }
        case NODE_SWIPER_EVENT_ON_CHANGE: {
            // The active page index (SelectionChanged) — reuse event kind 6.
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, 6, c ? (double)c->data[0].i32 : 0.0, "");
            break;
        }
        // Focus pair + text-input submit (docs/focus.md) — kinds match the Android bridge.
        case NODE_ON_FOCUS:
            day_arkui_on_event(id, 16, 1.0, "");
            break;
        case NODE_ON_BLUR:
            day_arkui_on_event(id, 16, 0.0, "");
            break;
        case NODE_TEXT_INPUT_ON_SUBMIT:
            day_arkui_on_event(id, 17, 0.0, "");
            break;
        default:
            break;
    }
}

// day node-kind → ArkUI_NodeType.
static ArkUI_NodeType kind_map(int32_t k) {
    switch (k) {
        case 0: return ARKUI_NODE_STACK;   // container (day owns absolute layout); also DIVIDER
        case 1: return ARKUI_NODE_TEXT;    // label
        case 2: return ARKUI_NODE_BUTTON;  // button
        case 3: return ARKUI_NODE_TEXT_INPUT;
        case 4: return ARKUI_NODE_TOGGLE;
        case 5: return ARKUI_NODE_SLIDER;
        case 6: return ARKUI_NODE_SCROLL;
        case 7: return ARKUI_NODE_COLUMN;
        case 8: return ARKUI_NODE_LOADING_PROGRESS;  // indeterminate spinner
        case 9: return ARKUI_NODE_IMAGE;  // image (by name, addressed via resource://RAWFILE)
        case 10: return ARKUI_NODE_CUSTOM;    // canvas (§11): custom node + on-draw callback
        case 11: return ARKUI_NODE_PROGRESS;  // determinate progress bar
        case 12: return ARKUI_NODE_SWIPER;    // tabs pager
        case 13: return ARKUI_NODE_LIST;      // recycling list (NodeAdapter)
        case 14: return ARKUI_NODE_LIST_ITEM; // one recycled list row
        case 15: return ARKUI_NODE_ROW;       // horizontal flow (menu rows: label + chevron)
        default: return ARKUI_NODE_STACK;
    }
}

static void set_str(void* n, ArkUI_NodeAttributeType a, const char* s) {
    ArkUI_AttributeItem it{};
    it.string = s ? s : "";
    g_api->setAttribute((ArkUI_NodeHandle)n, a, &it);
}
static void set_f32(void* n, ArkUI_NodeAttributeType a, float v) {
    ArkUI_NumberValue nv;
    nv.f32 = v;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, a, &it);
}
static void set_u32(void* n, ArkUI_NodeAttributeType a, uint32_t v) {
    ArkUI_NumberValue nv;
    nv.u32 = v;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, a, &it);
}

extern "C" {

// One-time: resolve the NodeAPI and register the global event receiver.
void day_ark_init(void) {
    if (!g_api) {
        OH_ArkUI_GetModuleInterface(ARKUI_NATIVE_NODE, ArkUI_NativeNodeAPI_1, g_api);
    }
    if (g_api) g_api->registerNodeEventReceiver(event_receiver);
}

void* day_ark_node_new(int32_t kind) {
    return g_api ? g_api->createNode(kind_map(kind)) : nullptr;
}
void day_ark_node_dispose(void* n) {
    if (g_api && n) g_api->disposeNode((ArkUI_NodeHandle)n);
}
void day_ark_add_child(void* p, void* c) {
    if (g_api) g_api->addChild((ArkUI_NodeHandle)p, (ArkUI_NodeHandle)c);
}
// Scroll axis for an ARKUI_NODE_SCROLL (docs/shapes.md h-scroll): horizontal vs the default
// vertical.
void day_ark_scroll_direction(void* n, int horizontal) {
    if (!g_api || !n) return;
    ArkUI_NumberValue nv;
    nv.i32 = horizontal ? ARKUI_SCROLL_DIRECTION_HORIZONTAL : ARKUI_SCROLL_DIRECTION_VERTICAL;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_SCROLL_SCROLL_DIRECTION, &it);
}
void day_ark_insert_child(void* p, void* c, int32_t pos) {
    if (g_api) g_api->insertChildAt((ArkUI_NodeHandle)p, (ArkUI_NodeHandle)c, pos);
}
void day_ark_remove_child(void* p, void* c) {
    if (g_api) g_api->removeChild((ArkUI_NodeHandle)p, (ArkUI_NodeHandle)c);
}

void day_ark_set_text(void* n, const char* s) { set_str(n, NODE_TEXT_CONTENT, s); }
void day_ark_set_button_label(void* n, const char* s) { set_str(n, NODE_BUTTON_LABEL, s); }
void day_ark_set_input_text(void* n, const char* s) { set_str(n, NODE_TEXT_INPUT_TEXT, s); }
void day_ark_set_placeholder(void* n, const char* s) { set_str(n, NODE_TEXT_INPUT_PLACEHOLDER, s); }
// NODE_IMAGE_SRC accepts a "resource://RAWFILE/<path>" URI | file path | network URL | base64.
void day_ark_set_image_src(void* n, const char* s) { set_str(n, NODE_IMAGE_SRC, s); }
// Scaling (§18.3): `fit` is an ArkUI_ObjectFit (CONTAIN=0 / COVER=1 / FILL=3).
void day_ark_set_image_fit(void* n, int32_t fit) {
    ArkUI_NumberValue nv;
    nv.i32 = fit;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_IMAGE_OBJECT_FIT, &it);
}
void day_ark_set_toggle(void* n, int32_t on) {
    ArkUI_NumberValue nv;
    nv.i32 = on ? 1 : 0;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_TOGGLE_VALUE, &it);
}
void day_ark_set_slider(void* n, double v) { set_f32(n, NODE_SLIDER_VALUE, (float)v); }

// Absolute layout (day owns it): position + explicit size, all in vp.
void day_ark_set_frame(void* n, double x, double y, double w, double h) {
    ArkUI_NumberValue pos[2];
    pos[0].f32 = (float)x;
    pos[1].f32 = (float)y;
    ArkUI_AttributeItem pit{};
    pit.value = pos;
    pit.size = 2;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_POSITION, &pit);
    set_f32(n, NODE_WIDTH, (float)w);
    set_f32(n, NODE_HEIGHT, (float)h);
}
void day_ark_set_bg_color(void* n, uint32_t argb) { set_u32(n, NODE_BACKGROUND_COLOR, argb); }
// Explicit size only (no NODE_POSITION) — for children whose parent owns their placement (Swiper).
void day_ark_set_size(void* n, double w, double h) {
    set_f32(n, NODE_WIDTH, (float)w);
    set_f32(n, NODE_HEIGHT, (float)h);
}
void day_ark_set_font_size(void* n, double vp) { set_f32(n, NODE_FONT_SIZE, (float)vp); }
void day_ark_set_font_color(void* n, uint32_t argb) { set_u32(n, NODE_FONT_COLOR, argb); }
// Bundled custom font family (§18.4) — registered from rawfile day/fonts.json by the
// platform/ohos scaffold's EntryAbility (ArkTS font.registerFont) before the native UI loads;
// ArkUI falls back to the default family when the name isn't registered.
void day_ark_set_font_family(void* n, const char* family) { set_str(n, NODE_FONT_FAMILY, family); }
void day_ark_set_corner_radius(void* n, double vp) { set_f32(n, NODE_BORDER_RADIUS, (float)vp); }

// Determinate progress bar: ArkUI uses a value in [0, total]; day passes the 0..1 fraction, so
// scale onto a fixed 0..1000 range (like day-android's LinearProgressIndicator ticks).
void day_ark_set_progress(void* n, double fraction) {
    set_f32(n, NODE_PROGRESS_TOTAL, 1000.0f);
    float v = (float)(fraction < 0 ? 0 : fraction > 1 ? 1 : fraction) * 1000.0f;
    set_f32(n, NODE_PROGRESS_VALUE, v);
}

// Visibility: 0 = VISIBLE, else NONE (removed from layout — used to show one TABS page at a time).
void day_ark_set_visibility(void* n, int32_t visible) {
    ArkUI_NumberValue nv;
    nv.i32 = visible ? ARKUI_VISIBILITY_VISIBLE : ARKUI_VISIBILITY_NONE;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_VISIBILITY, &it);
}

// The active tab/page index for a Swiper (NODE_SWIPER_INDEX).
void day_ark_set_swiper_index(void* n, int32_t i) {
    ArkUI_NumberValue nv;
    nv.i32 = i;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_SWIPER_INDEX, &it);
}

// Configure a Swiper used as a tab pager: show the dot indicator, don't loop.
void day_ark_swiper_setup(void* n) {
    ArkUI_NumberValue ind[1];
    ind[0].i32 = 1; // show indicator
    ArkUI_AttributeItem iit{};
    iit.value = ind;
    iit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_SWIPER_SHOW_INDICATOR, &iit);
    ArkUI_NumberValue loop[1];
    loop[0].i32 = 0; // no wraparound
    ArkUI_AttributeItem lit{};
    lit.value = loop;
    lit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_SWIPER_LOOP, &lit);
}

// Accessibility (§13): NODE_ACCESSIBILITY_TEXT is the label a screen reader announces;
// hidden removes the node (and its subtree) from the accessibility tree via NODE_ACCESSIBILITY_MODE.
void day_ark_set_a11y(void* n, const char* label, int32_t hidden) {
    if (label && *label) set_str(n, NODE_ACCESSIBILITY_TEXT, label);
    ArkUI_NumberValue nv;
    // ArkUI_AccessibilityMode: 0 = AUTO, 1 = ENABLED, 2 = DISABLED, 3 = DISABLED_FOR_DESCENDANTS.
    nv.i32 = hidden ? 3 : 0;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_ACCESSIBILITY_MODE, &it);
}

// Measure a node under a width/height proposal (<=0 means "unbounded"); result in vp.
void day_ark_measure(void* n, double max_w, double max_h, double* out_w, double* out_h) {
    *out_w = 0;
    *out_h = 0;
    if (!g_api || !n) return;
    ArkUI_LayoutConstraint* c = OH_ArkUI_LayoutConstraint_Create();
    int32_t mw = max_w > 0 ? (int32_t)(max_w * g_density) : 1000000;
    int32_t mh = max_h > 0 ? (int32_t)(max_h * g_density) : 1000000;
    OH_ArkUI_LayoutConstraint_SetMaxWidth(c, mw);
    OH_ArkUI_LayoutConstraint_SetMaxHeight(c, mh);
    OH_ArkUI_LayoutConstraint_SetMinWidth(c, 0);
    OH_ArkUI_LayoutConstraint_SetMinHeight(c, 0);
    g_api->measureNode((ArkUI_NodeHandle)n, c);
    ArkUI_IntSize sz = g_api->getMeasuredSize((ArkUI_NodeHandle)n);
    OH_ArkUI_LayoutConstraint_Dispose(c);
    *out_w = sz.width / g_density;
    *out_h = sz.height / g_density;
}

// kind: 0=click 1=text-change 2=toggle-change 3=slider-change. `id` is delivered back as userData.
void day_ark_register_event(void* n, int32_t kind, uint64_t id) {
    if (!g_api) return;
    ArkUI_NodeEventType t;
    switch (kind) {
        case 0: t = NODE_ON_CLICK; break;
        case 1: t = NODE_TEXT_INPUT_ON_CHANGE; break;
        case 2: t = NODE_TOGGLE_ON_CHANGE; break;
        case 3: t = NODE_SLIDER_EVENT_ON_CHANGE; break;
        case 6: t = NODE_SWIPER_EVENT_ON_CHANGE; break;
        default: return;
    }
    g_api->registerNodeEvent((ArkUI_NodeHandle)n, t, 0, (void*)(uintptr_t)id);
}

// Focus (docs/focus.md): observe gain/blur (+ the text-input submit action) on the node.
void day_ark_enable_focus(void* n, uint64_t id, int32_t is_text_input) {
    if (!g_api) return;
    auto h = (ArkUI_NodeHandle)n;
    g_api->registerNodeEvent(h, NODE_ON_FOCUS, 0, (void*)(uintptr_t)id);
    g_api->registerNodeEvent(h, NODE_ON_BLUR, 0, (void*)(uintptr_t)id);
    if (is_text_input)
        g_api->registerNodeEvent(h, NODE_TEXT_INPUT_ON_SUBMIT, 0, (void*)(uintptr_t)id);
}

// Drive focus: request it (typed errors for non-focusable targets are deliberately ignored —
// no event means the signal snaps back, docs/focus.md rule 2), or clear the UI context's
// focus — only while this node still owns it, so a stale release can't blur a sibling.
void day_ark_focus(void* n, int32_t focused) {
    if (!g_api) return;
    auto h = (ArkUI_NodeHandle)n;
    if (focused) {
        (void)OH_ArkUI_FocusRequest(h);
    } else {
        const ArkUI_AttributeItem* st = g_api->getAttribute(h, NODE_FOCUS_STATUS);
        bool owns = st && st->value && st->size > 0 && st->value[0].i32 != 0;
        if (!owns) return;
        ArkUI_ContextHandle ctx = OH_ArkUI_GetContextByNode(h);
        if (ctx) OH_ArkUI_FocusClear(ctx);
    }
}

int32_t day_ark_content_add(void* content, void* node) {
    return OH_ArkUI_NodeContent_AddNode((ArkUI_NodeContentHandle)content, (ArkUI_NodeHandle)node);
}

void day_ark_post(void (*cb)(void*), void* data) {
    {
        std::lock_guard<std::mutex> lk(g_mtx);
        g_queue.push_back({cb, data});
    }
    if (g_async_ready) uv_async_send(&g_async);
}

double day_ark_density(void) { return g_density; }

// Ask the ArkTS-registered picker to open/save a file. Runs on the JS thread, so a plain
// napi_call_function is safe. Falls back to an immediate cancel if nothing is registered.
void day_ark_present_file(uint64_t req, int32_t mode, const char* name, const char* src,
                          const char* filters) {
    if (!g_env || !g_file_picker) {
        day_arkui_on_event(req, 5, 0.0, ""); // cancel (no picker)
        return;
    }
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_file_picker, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value args[5];
        napi_create_double(g_env, (double)req, &args[0]);
        napi_create_int32(g_env, mode, &args[1]);
        napi_create_string_utf8(g_env, name ? name : "", NAPI_AUTO_LENGTH, &args[2]);
        napi_create_string_utf8(g_env, src ? src : "", NAPI_AUTO_LENGTH, &args[3]);
        napi_create_string_utf8(g_env, filters ? filters : "", NAPI_AUTO_LENGTH, &args[4]);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 5, args, &ret);
    } else {
        day_arkui_on_event(req, 5, 0.0, "");
    }
    napi_close_handle_scope(g_env, scope);
}

// Push one Day page into the ArkTS Navigation: asks the registered push callback for a fresh
// NodeContent (the callback also pushes the NavDestination onto the NavPathStack) and mounts
// the page's native node into it. JS thread only. Returns 0 on success.
int32_t day_ark_nav_push(void* page, uint64_t key, const char* title) {
    if (!g_env || !g_nav_push) return -1;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    int32_t rc = -1;
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_nav_push, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value args[2];
        napi_create_double(g_env, (double)key, &args[0]);
        napi_create_string_utf8(g_env, title ? title : "", NAPI_AUTO_LENGTH, &args[1]);
        napi_value ret = nullptr;
        if (napi_call_function(g_env, undef, cb, 2, args, &ret) == napi_ok && ret) {
            ArkUI_NodeContentHandle content = nullptr;
            OH_ArkUI_GetNodeContentFromNapiValue(g_env, ret, &content);
            if (content) {
                napi_ref ref = nullptr;
                napi_create_reference(g_env, ret, 1, &ref);
                g_nav_contents[key] = DayNavContent{content, ref};
                OH_ArkUI_NodeContent_AddNode(content, (ArkUI_NodeHandle)page);
                // Re-homed subtrees keep their (already clean) layout/render state, and the
                // fresh NavDestination composes an EMPTY content layer over the previous page
                // unless the attached tree is explicitly re-marked for layout + paint.
                if (g_api) {
                    g_api->markDirty((ArkUI_NodeHandle)page, NODE_NEED_MEASURE);
                    g_api->markDirty((ArkUI_NodeHandle)page, NODE_NEED_LAYOUT);
                    g_api->markDirty((ArkUI_NodeHandle)page, NODE_NEED_RENDER);
                }
                rc = 0;
            }
        }
    }
    napi_close_handle_scope(g_env, scope);
    return rc;
}

// Pop the top NavDestination (Day-initiated: programmatic route change). JS thread only.
void day_ark_nav_pop(void) {
    if (!g_env || !g_nav_pop) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_nav_pop, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 0, nullptr, &ret);
    }
    napi_close_handle_scope(g_env, scope);
}

// Retitle the top destination (NavPatch::Title). JS thread only.
void day_ark_nav_set_title(const char* title) {
    if (!g_env || !g_nav_title) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_nav_title, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value arg;
        napi_create_string_utf8(g_env, title ? title : "", NAPI_AUTO_LENGTH, &arg);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 1, &arg, &ret);
    }
    napi_close_handle_scope(g_env, scope);
}

// Unmount a page's node from its still-LIVE NodeContent (a Day-initiated pop detaches before
// the destination's teardown) and release the slot. JS thread only.
void day_ark_nav_remove(uint64_t key, void* page) {
    auto it = g_nav_contents.find(key);
    if (it != g_nav_contents.end()) {
        OH_ArkUI_NodeContent_RemoveNode(it->second.content, (ArkUI_NodeHandle)page);
        if (g_env && it->second.ref) napi_delete_reference(g_env, it->second.ref);
        g_nav_contents.erase(it);
    }
}

// Release a slot whose NavDestination ALREADY disappeared (native back / reported pop): the
// destination tore its content down, so touching the nodes again would use freed memory —
// just drop the bookkeeping and the keep-alive ref. JS thread only.
void day_ark_nav_forget(uint64_t key) {
    auto it = g_nav_contents.find(key);
    if (it != g_nav_contents.end()) {
        if (g_env && it->second.ref) napi_delete_reference(g_env, it->second.ref);
        g_nav_contents.erase(it);
    }
}

// ---- bundled data resources (§18.3): app rawfile store ---------------------
// Opaque cleanup token handed back to Rust for a single opened resource view.
struct DayResMap {
    void* base;    // munmap base (page-aligned) when mmap'd, else the malloc'd buffer
    size_t maplen; // length passed to munmap; 0 marks a heap copy (free `base` instead)
};

int32_t day_ark_res_available(void) { return g_res_mgr ? 1 : 0; }

// Open rawfile `path` (e.g. "day/numbers.bin") and expose its bytes. Prefers a zero-copy mmap of the
// uncompressed entry inside the .hap (via OH_ResourceManager_GetRawFileDescriptor → {fd,start,
// length}); falls back to reading the whole file into a heap buffer if the descriptor/mmap is
// unavailable. See rawfile/raw_file.h + rawfile/raw_file_manager.h.
int32_t day_ark_res_open(const char* path, const uint8_t** out_data, size_t* out_len,
                         void** out_handle) {
    *out_data = nullptr;
    *out_len = 0;
    *out_handle = nullptr;
    if (!g_res_mgr || !path) return 0;
    RawFile* rf = OH_ResourceManager_OpenRawFile(g_res_mgr, path);
    if (!rf) return 0;

    // Zero-copy path: the CLI stages resources uncompressed, so the entry has a real fd/offset/length
    // inside the .hap we can mmap (the 32-bit descriptor takes the same RawFile* we opened; its
    // long fields are 64-bit on the ohos targets). The offset need not be page-aligned, so align down
    // and bias the returned pointer. Note the OH getters take the descriptor by C++ reference.
    RawFileDescriptor fd{};
    bool have_fd = OH_ResourceManager_GetRawFileDescriptor(rf, fd);
    if (have_fd && fd.fd >= 0 && fd.length > 0) {
        long page = sysconf(_SC_PAGESIZE);
        long misalign = page > 0 ? (fd.start % page) : 0;
        size_t map_len = (size_t)(fd.length + misalign);
        void* base = mmap(nullptr, map_len, PROT_READ, MAP_PRIVATE, fd.fd, fd.start - misalign);
        // The descriptor owns a dup'd fd; release it — the mapping survives the close.
        OH_ResourceManager_ReleaseRawFileDescriptor(fd);
        OH_ResourceManager_CloseRawFile(rf);
        if (base != MAP_FAILED) {
            *out_data = (const uint8_t*)base + misalign;
            *out_len = (size_t)fd.length;
            *out_handle = new DayResMap{base, map_len};
            return 1;
        }
        // mmap failed → reopen and fall through to the heap-copy path below.
        rf = OH_ResourceManager_OpenRawFile(g_res_mgr, path);
        if (!rf) return 0;
    } else if (have_fd) {
        // Descriptor obtained but unusable — release it so the dup'd fd isn't leaked.
        OH_ResourceManager_ReleaseRawFileDescriptor(fd);
    }

    // Fallback copy path: read the whole file into a heap buffer.
    long size = OH_ResourceManager_GetRawFileSize(rf);
    if (size <= 0) {
        OH_ResourceManager_CloseRawFile(rf);
        return 0;
    }
    void* buf = malloc((size_t)size);
    if (!buf) {
        OH_ResourceManager_CloseRawFile(rf);
        return 0;
    }
    int read = OH_ResourceManager_ReadRawFile(rf, buf, (size_t)size);
    OH_ResourceManager_CloseRawFile(rf);
    if (read <= 0) {
        free(buf);
        return 0;
    }
    *out_data = (const uint8_t*)buf;
    *out_len = (size_t)read;
    *out_handle = new DayResMap{buf, 0};
    return 1;
}

void day_ark_res_close(void* handle) {
    DayResMap* tok = (DayResMap*)handle;
    if (!tok) return;
    if (tok->maplen) munmap(tok->base, tok->maplen);
    else free(tok->base);
    delete tok;
}

} // extern "C"

// ---- canvas (§11): ARKUI_NODE_CUSTOM + on-draw via OH_Drawing --------------
// day records a display list in day points (vp); the custom node's draw canvas is in px, so we push
// a density scale first. The op encoding mirrors day_spec::encode_ops / DayCanvasView.java: 9 doubles
// per op [kind,a,b,c,d,e,f,g,argb], with polygon points riding the 0x1F-joined text channel.
struct CanvasOps {
    std::vector<double> nums;
    std::vector<std::string> texts;
};
static std::map<void*, CanvasOps> g_canvas; // custom node → its ops
static const int32_t CANVAS_DRAW_TARGET = 77;

static uint32_t argb_to_drawing(double bits) {
    return (uint32_t)(int64_t)bits; // already 0xAARRGGBB
}

// Split the 0x1F-joined text channel into fields.
static std::vector<std::string> split_texts(const std::string& joined) {
    std::vector<std::string> out;
    size_t start = 0;
    if (joined.empty()) return out;
    for (size_t i = 0; i <= joined.size(); i++) {
        if (i == joined.size() || joined[i] == '\x1f') {
            out.push_back(joined.substr(start, i - start));
            start = i + 1;
        }
    }
    return out;
}

// A decoded kind-14 record (set-gradient): type (0 linear, 1 radial) + unit geometry + stops,
// applied as the brush's shader effect for the NEXT fill-shape record (resolved against that
// shape's bounds).
struct PendingGradient {
    bool active = false;
    int kind = 0;
    float sx = 0, sy = 0, ex = 0, ey = 0; // linear: start/end unit points; radial: sx,sy=center, ex=radius
    std::vector<uint32_t> colors;
    std::vector<float> offsets;
};

static void apply_gradient(OH_Drawing_Brush* brush, PendingGradient& g,
                           float x, float y, float w, float h) {
    OH_Drawing_ShaderEffect* fx = nullptr;
    if (g.kind == 1) {
        // Radial, elliptical-to-bounds: circular in unit space, stretched onto the bounds by
        // the shader's local matrix (the same rule as every other backend).
        OH_Drawing_Point2D center{ g.sx, g.sy };
        OH_Drawing_Matrix* m = OH_Drawing_MatrixCreate();
        OH_Drawing_MatrixSetMatrix(m, w, 0, x, 0, h, y, 0, 0, 1);
        fx = OH_Drawing_ShaderEffectCreateRadialGradientWithLocalMatrix(
            &center, g.ex > 1e-4f ? g.ex : 1e-4f, g.colors.data(), g.offsets.data(),
            (uint32_t)g.colors.size(), CLAMP, m);
        OH_Drawing_MatrixDestroy(m);
    } else {
        OH_Drawing_Point* start = OH_Drawing_PointCreate(x + g.sx * w, y + g.sy * h);
        OH_Drawing_Point* end = OH_Drawing_PointCreate(x + g.ex * w, y + g.ey * h);
        fx = OH_Drawing_ShaderEffectCreateLinearGradient(
            start, end, g.colors.data(), g.offsets.data(), (uint32_t)g.colors.size(), CLAMP);
        OH_Drawing_PointDestroy(start);
        OH_Drawing_PointDestroy(end);
    }
    OH_Drawing_BrushSetShaderEffect(brush, fx);
    OH_Drawing_ShaderEffectDestroy(fx);
    g.active = false;
}

static void canvas_draw(void* node, OH_Drawing_Canvas* cv) {
    auto it = g_canvas.find(node);
    if (it == g_canvas.end()) return;
    const std::vector<double>& n = it->second.nums;
    const std::vector<std::string>& texts = it->second.texts;
    OH_Drawing_Pen* pen = OH_Drawing_PenCreate();
    OH_Drawing_PenSetAntiAlias(pen, true);
    OH_Drawing_Brush* brush = OH_Drawing_BrushCreate();
    OH_Drawing_BrushSetAntiAlias(brush, true);

    // Base transform: scale vp → px so day's point-space ops land correctly.
    OH_Drawing_CanvasSave(cv);
    OH_Drawing_Matrix* scale = OH_Drawing_MatrixCreate();
    float d = (float)g_density;
    OH_Drawing_MatrixSetMatrix(scale, d, 0, 0, 0, d, 0, 0, 0, 1);
    OH_Drawing_CanvasConcatMatrix(cv, scale);

    size_t text_i = 0;
    PendingGradient grad;
    for (size_t i = 0; i + 8 < n.size(); i += 9) {
        int kind = (int)n[i];
        float a = (float)n[i + 1], b = (float)n[i + 2], c = (float)n[i + 3], dd = (float)n[i + 4];
        float e = (float)n[i + 5], f = (float)n[i + 6], g = (float)n[i + 7];
        uint32_t col = argb_to_drawing(n[i + 8]);
        OH_Drawing_PenSetColor(pen, col);
        OH_Drawing_PenSetWidth(pen, g > 0 ? g : 1.0f);
        OH_Drawing_BrushSetColor(brush, col);
        bool stroke = (kind == 1 || kind == 4 || kind == 5 || kind == 6 || kind == 12 || kind == 13);
        // Fill kinds consume a pending gradient (kind 14) as the brush's shader effect.
        if (grad.active) {
            switch (kind) {
                case 0: case 2: case 3:
                    apply_gradient(brush, grad, a, b, c, dd);
                    break;
                default:
                    break; // kind 11 resolves after its points parse (bounds unknown here)
            }
        } else {
            OH_Drawing_BrushSetShaderEffect(brush, nullptr);
        }
        if (stroke) OH_Drawing_CanvasAttachPen(cv, pen);
        else OH_Drawing_CanvasAttachBrush(cv, brush);
        switch (kind) {
            case 0:
            case 1: { // rect fill / stroke
                OH_Drawing_Rect* r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                OH_Drawing_CanvasDrawRect(cv, r);
                OH_Drawing_RectDestroy(r);
                break;
            }
            case 2:
            case 13: { // rounded rect fill / stroke (radius = e)
                OH_Drawing_Rect* r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                OH_Drawing_RoundRect* rr = OH_Drawing_RoundRectCreate(r, e, e);
                OH_Drawing_CanvasDrawRoundRect(cv, rr);
                OH_Drawing_RoundRectDestroy(rr);
                OH_Drawing_RectDestroy(r);
                break;
            }
            case 3:
            case 4: { // ellipse fill / stroke
                OH_Drawing_Rect* r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                OH_Drawing_CanvasDrawOval(cv, r);
                OH_Drawing_RectDestroy(r);
                break;
            }
            case 5: { // arc (start=e sweep=f)
                OH_Drawing_Rect* r = OH_Drawing_RectCreate(a, b, a + c, b + dd);
                OH_Drawing_CanvasDrawArc(cv, r, e, f);
                OH_Drawing_RectDestroy(r);
                break;
            }
            case 6: // line from (a,b) to (c,d)
                OH_Drawing_CanvasDrawLine(cv, a, b, c, dd);
                break;
            case 7: { // text: size=e, anchor=f (0 leading, 1 centered); string on the text channel
                std::string s = text_i < texts.size() ? texts[text_i++] : std::string();
                OH_Drawing_Font* font = OH_Drawing_FontCreate();
                OH_Drawing_FontSetTextSize(font, e);
                OH_Drawing_TextBlob* blob = OH_Drawing_TextBlobCreateFromString(
                    s.c_str(), font, TEXT_ENCODING_UTF8);
                float x = a;
                if (f == 1.0f) x = a - (float)s.size() * e * 0.28f; // rough centering
                OH_Drawing_CanvasDrawTextBlob(cv, blob, x, b);
                OH_Drawing_TextBlobDestroy(blob);
                OH_Drawing_FontDestroy(font);
                break;
            }
            case 8:
                OH_Drawing_CanvasSave(cv);
                break;
            case 9:
                OH_Drawing_CanvasRestore(cv);
                break;
            case 10: { // concat affine [a b c d tx ty] (day_geometry::Affine, column vectors)
                OH_Drawing_Matrix* m = OH_Drawing_MatrixCreate();
                OH_Drawing_MatrixSetMatrix(m, a, c, e, b, dd, f, 0, 0, 1);
                OH_Drawing_CanvasConcatMatrix(cv, m);
                OH_Drawing_MatrixDestroy(m);
                break;
            }
            case 11:
            case 12: { // polygon fill / stroke — points ride the text channel as "x,y x,y …"
                std::string pts = text_i < texts.size() ? texts[text_i++] : std::string();
                OH_Drawing_Path* path = OH_Drawing_PathCreate();
                bool first = true;
                size_t p = 0;
                while (p < pts.size()) {
                    size_t sp = pts.find(' ', p);
                    std::string tok = pts.substr(p, sp == std::string::npos ? sp : sp - p);
                    size_t comma = tok.find(',');
                    if (comma != std::string::npos) {
                        float px = strtof(tok.substr(0, comma).c_str(), nullptr);
                        float py = strtof(tok.substr(comma + 1).c_str(), nullptr);
                        if (first) { OH_Drawing_PathMoveTo(path, px, py); first = false; }
                        else OH_Drawing_PathLineTo(path, px, py);
                    }
                    if (sp == std::string::npos) break;
                    p = sp + 1;
                }
                OH_Drawing_PathClose(path);
                if (kind == 11 && grad.active) {
                    OH_Drawing_Rect* pb = OH_Drawing_RectCreate(0, 0, 0, 0);
                    OH_Drawing_PathGetBounds(path, pb);
                    float bx = OH_Drawing_RectGetLeft(pb), by = OH_Drawing_RectGetTop(pb);
                    float bw = OH_Drawing_RectGetWidth(pb), bh = OH_Drawing_RectGetHeight(pb);
                    OH_Drawing_RectDestroy(pb);
                    OH_Drawing_CanvasDetachBrush(cv);
                    apply_gradient(brush, grad, bx, by, bw, bh);
                    OH_Drawing_CanvasAttachBrush(cv, brush);
                }
                OH_Drawing_CanvasDrawPath(cv, path);
                OH_Drawing_PathDestroy(path);
                break;
            }
            case 14: { // set-gradient (f = type): stops ride texts as "offset,aarrggbb offset,aarrggbb ..."
                std::string stops = text_i < texts.size() ? texts[text_i++] : std::string();
                grad.kind = (int)f;
                grad.colors.clear();
                grad.offsets.clear();
                size_t p = 0;
                while (p < stops.size()) {
                    size_t sp = stops.find(' ', p);
                    std::string tok = stops.substr(p, sp == std::string::npos ? sp : sp - p);
                    size_t comma = tok.find(',');
                    if (comma != std::string::npos && comma > 0) {
                        grad.offsets.push_back(strtof(tok.substr(0, comma).c_str(), nullptr));
                        grad.colors.push_back((uint32_t)strtoul(tok.substr(comma + 1).c_str(), nullptr, 16));
                    }
                    if (sp == std::string::npos) break;
                    p = sp + 1;
                }
                grad.sx = a; grad.sy = b; grad.ex = c; grad.ey = dd;
                grad.active = grad.colors.size() >= 2;
                break;
            }
            default:
                break;
        }
        if (stroke) OH_Drawing_CanvasDetachPen(cv);
        else OH_Drawing_CanvasDetachBrush(cv);
    }
    OH_Drawing_CanvasRestore(cv);
    OH_Drawing_MatrixDestroy(scale);
    OH_Drawing_PenDestroy(pen);
    OH_Drawing_BrushDestroy(brush);
}

static void canvas_custom_receiver(ArkUI_NodeCustomEvent* ev) {
    if (!ev) return;
    if (OH_ArkUI_NodeCustomEvent_GetEventType(ev) != ARKUI_NODE_CUSTOM_EVENT_ON_DRAW) return;
    void* node = OH_ArkUI_NodeCustomEvent_GetUserData(ev);
    ArkUI_DrawContext* dc = OH_ArkUI_NodeCustomEvent_GetDrawContextInDraw(ev);
    if (!dc) return;
    OH_Drawing_Canvas* cv = (OH_Drawing_Canvas*)OH_ArkUI_DrawContext_GetCanvas(dc);
    if (cv) canvas_draw(node, cv);
}

extern "C" {

// Register the on-draw custom-event receiver for a canvas custom node.
void day_ark_canvas_init(void* node) {
    if (!g_api || !node) return;
    g_api->addNodeCustomEventReceiver((ArkUI_NodeHandle)node, canvas_custom_receiver);
    g_api->registerNodeCustomEvent((ArkUI_NodeHandle)node, ARKUI_NODE_CUSTOM_EVENT_ON_DRAW,
                                   CANVAS_DRAW_TARGET, node);
}

// Store the encoded display list for `node` and request a repaint.
void day_ark_set_canvas_ops(void* node, const double* nums, uint32_t count, const char* texts) {
    CanvasOps ops;
    ops.nums.assign(nums, nums + count);
    ops.texts = split_texts(texts ? texts : "");
    g_canvas[node] = std::move(ops);
    if (g_api) g_api->markDirty((ArkUI_NodeHandle)node, NODE_NEED_RENDER);
}

// ---- recycling list: ARKUI_NODE_LIST + a NodeAdapter -----------------------
// A cell is a LIST_ITEM wrapping an inner Stack that day mounts the row subtree into. Cells scrolled
// out of view are pushed to a REUSE POOL rather than disposed — so the inner Stack pointer stays
// stable and day-core's cell cache rebinds it (day's `recycle` is a no-op; cells "stay cached").
struct DayList {
    ArkUI_NodeAdapterHandle adapter;
    uint64_t host_id;
    float row_h; // px; 0 = content-sized
    std::vector<ArkUI_NodeHandle> pool;
};
static std::map<void*, DayList*> g_lists; // list node → its adapter binding

static void list_adapter_receiver(ArkUI_NodeAdapterEvent* ev) {
    auto* dl = (DayList*)OH_ArkUI_NodeAdapterEvent_GetUserData(ev);
    if (!dl) return;
    switch (OH_ArkUI_NodeAdapterEvent_GetType(ev)) {
        case NODE_ADAPTER_EVENT_ON_GET_NODE_ID:
            OH_ArkUI_NodeAdapterEvent_SetNodeId(ev, OH_ArkUI_NodeAdapterEvent_GetItemIndex(ev));
            break;
        case NODE_ADAPTER_EVENT_ON_ADD_NODE_TO_ADAPTER: {
            uint32_t idx = OH_ArkUI_NodeAdapterEvent_GetItemIndex(ev);
            ArkUI_NodeHandle cell;
            if (!dl->pool.empty()) {
                cell = dl->pool.back();
                dl->pool.pop_back();
            } else {
                cell = g_api->createNode(ARKUI_NODE_LIST_ITEM);
                ArkUI_NodeHandle inner = g_api->createNode(ARKUI_NODE_STACK);
                if (dl->row_h > 0) {
                    set_f32(cell, NODE_HEIGHT, dl->row_h / g_density);
                    set_f32(inner, NODE_HEIGHT, dl->row_h / g_density);
                }
                g_api->addChild(cell, inner);
            }
            ArkUI_NodeHandle inner = g_api->getChildAt(cell, 0);
            day_arkui_list_bind(dl->host_id, idx, inner); // build (fresh) or rebind (recycled)
            OH_ArkUI_NodeAdapterEvent_SetItem(ev, cell);
            break;
        }
        case NODE_ADAPTER_EVENT_ON_REMOVE_NODE_FROM_ADAPTER: {
            // Return the cell to the pool for reuse; keep the inner Stack + day's cache intact.
            ArkUI_NodeHandle removed = OH_ArkUI_NodeAdapterEvent_GetRemovedNode(ev);
            if (removed) dl->pool.push_back(removed);
            break;
        }
        default:
            break;
    }
}

void day_ark_list_init(void* node, uint64_t host_id, double row_h_vp) {
    if (!g_api || !node) return;
    DayList* dl = new DayList{OH_ArkUI_NodeAdapter_Create(), host_id,
                              (float)(row_h_vp * g_density), {}};
    OH_ArkUI_NodeAdapter_RegisterEventReceiver(dl->adapter, dl, list_adapter_receiver);
    g_lists[node] = dl;
    ArkUI_AttributeItem it{};
    it.object = dl->adapter;
    g_api->setAttribute((ArkUI_NodeHandle)node, NODE_LIST_NODE_ADAPTER, &it);
}

// Re-query the row count (the adapter re-fetches its visible cells).
void day_ark_list_reload(void* node) {
    auto it = g_lists.find(node);
    if (it == g_lists.end()) return;
    uint32_t count = day_arkui_list_count(it->second->host_id);
    OH_ArkUI_NodeAdapter_SetTotalNodeCount(it->second->adapter, count);
}

// Scroll the list so its last row is fully visible (docs/list.md, chat "stick to bottom").
void day_ark_list_scroll_to_end(void* node) {
    auto it = g_lists.find(node);
    if (it == g_lists.end()) return;
    int32_t last = (int32_t)OH_ArkUI_NodeAdapter_GetTotalNodeCount(it->second->adapter) - 1;
    if (last < 0) return;
    ArkUI_NumberValue v[1];
    v[0].i32 = last;
    ArkUI_AttributeItem item{};
    item.value = v;
    item.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)node, NODE_LIST_SCROLL_TO_INDEX, &item);
}

// A NAV_MENU / tab-bar row: full width, fixed height, left-aligned text with padding.
// Flex-grow within a Row/Column (the menu label grows so the chevron hugs the trailing edge).
void day_ark_set_flex_grow(void* n, double g) {
    ArkUI_NumberValue v;
    v.f32 = (float)g;
    ArkUI_AttributeItem it{};
    it.value = &v;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_FLEX_GROW, &it);
}

// A conventional list separator: full-width hairline; the caller picks the theme-aware color.
void day_ark_menu_separator(void* n, uint32_t argb) {
    ArkUI_NumberValue wp[1];
    wp[0].f32 = 1.0f;
    ArkUI_AttributeItem wit{};
    wit.value = wp;
    wit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_WIDTH_PERCENT, &wit);
    set_f32(n, NODE_HEIGHT, 0.7f);
    set_u32(n, NODE_BACKGROUND_COLOR, argb);
}

void day_ark_style_row(void* n, double height_vp) {
    ArkUI_NumberValue wp[1];
    wp[0].f32 = 1.0f; // 100% of the parent width
    ArkUI_AttributeItem wit{};
    wit.value = wp;
    wit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_WIDTH_PERCENT, &wit);
    set_f32(n, NODE_HEIGHT, (float)height_vp);
    ArkUI_NumberValue pad[1];
    pad[0].f32 = 16.0f;
    ArkUI_AttributeItem pit{};
    pit.value = pad;
    pit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_PADDING, &pit);
    // NODE_TEXT_ALIGN: 0 = START (left).
    ArkUI_NumberValue ta[1];
    ta[0].i32 = 0;
    ArkUI_AttributeItem tit{};
    tit.value = ta;
    tit.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_TEXT_ALIGN, &tit);
}

} // extern "C"

// Read a NAPI string argument into a std::string (queries the exact length first).
static std::string napi_to_string(napi_env env, napi_value v) {
    size_t need = 0;
    if (napi_get_value_string_utf8(env, v, nullptr, 0, &need) != napi_ok) return std::string();
    std::string out(need, '\0');
    size_t written = 0;
    napi_get_value_string_utf8(env, v, &out[0], need + 1, &written);
    out.resize(written);
    return out;
}

// ---- NAPI module -----------------------------------------------------------
// ArkTS calls `start(nodeContent, widthVp, heightVp, density)` on the imported native module.
static napi_value DayStart(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;

    uv_loop_t* loop = nullptr;
    napi_get_uv_event_loop(env, &loop);
    if (loop && !g_async_ready) {
        uv_async_init(loop, &g_async, drain_async);
        g_async_ready = true;
    }

    ArkUI_NodeContentHandle content = nullptr;
    OH_ArkUI_GetNodeContentFromNapiValue(env, argv[0], &content);
    double w = 0, h = 0, dens = 1.0;
    napi_get_value_double(env, argv[1], &w);
    napi_get_value_double(env, argv[2], &h);
    napi_get_value_double(env, argv[3], &dens);
    g_density = dens > 0 ? dens : 1.0;

    day_ark_init();
    day_arkui_start(content, w, h, g_density);

    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// ArkTS registers its file picker + the app cache dir (docs/files.md): `registerFilePicker(cb,
// cacheDir)`. `cb` is `(req, mode, name, src, filters) => void` and answers via `onFileResult`.
static napi_value RegisterFilePicker(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    if (g_file_picker) {
        napi_delete_reference(env, g_file_picker);
        g_file_picker = nullptr;
    }
    napi_create_reference(env, argv[0], 1, &g_file_picker);
    std::string cache = napi_to_string(env, argv[1]);
    if (!cache.empty()) day_arkui_set_cache_dir(cache.c_str());
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// ArkTS registers its URL opener: `registerOpenUrl(cb)`, `cb` = `(url: string) => void`
// (typically `context.startAbility({ action: 'ohos.want.action.viewData', uri: url })`).
static napi_value RegisterOpenUrl(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    if (g_open_url) {
        napi_delete_reference(env, g_open_url);
        g_open_url = nullptr;
    }
    if (argv[0]) napi_create_reference(env, argv[0], 1, &g_open_url);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// Open `url` in the system's default handler via the ArkTS opener. JS thread only. No-op when the
// app hasn't registered one.
void day_ark_open_url(const char* url) {
    if (!g_env || !g_open_url) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_open_url, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value arg;
        napi_create_string_utf8(g_env, url ? url : "", NAPI_AUTO_LENGTH, &arg);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 1, &arg, &ret);
    }
    napi_close_handle_scope(g_env, scope);
}

// ArkTS hands the app's resourceManager to native so the rawfile data-resource opener (§18.3) can
// read staged assets: `registerResourceManager(getContext(this).resourceManager)`. OH_ResourceManager
// _InitNativeResourceManager needs this ArkTS object — there is no native-only way to obtain it — so
// the entry ability must call this once (additive, like registerFilePicker; harmless if omitted, in
// which case `resource(name)` returns None and no data resources are available).
static napi_value RegisterResourceManager(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    if (argv[0]) {
        if (g_res_mgr) {
            OH_ResourceManager_ReleaseNativeResourceManager(g_res_mgr);
            g_res_mgr = nullptr;
        }
        g_res_mgr = OH_ResourceManager_InitNativeResourceManager(env, argv[0]);
    }
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// The picker's answer: `onFileResult(req, path)` — empty path = cancel (docs/files.md).
static napi_value OnFileResult(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double reqd = 0;
    napi_get_value_double(env, argv[0], &reqd);
    std::string path = napi_to_string(env, argv[1]);
    day_arkui_on_event((uint64_t)reqd, 5, 0.0, path.c_str()); // kind 5 = present files
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// ArkTS registers its Navigation bridge: `registerNav(push, pop, setTitle)` — see the
// Navigation-bridge comment at the top. Re-registration replaces the callbacks.
static napi_value RegisterNav(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    napi_ref* refs[3] = {&g_nav_push, &g_nav_pop, &g_nav_title};
    for (size_t i = 0; i < 3; i++) {
        if (*refs[i]) {
            napi_delete_reference(env, *refs[i]);
            *refs[i] = nullptr;
        }
        if (i < argc && argv[i]) napi_create_reference(env, argv[i], 1, refs[i]);
    }
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// A NavDestination disappeared (system back gesture, title-bar back button, or a Day-initiated
// pop finishing): `navPopped(key)`.
static napi_value NavPopped(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double key = 0;
    napi_get_value_double(env, argv[0], &key);
    day_arkui_nav_popped((uint64_t)key);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// A destination's content area (vp): `navPageArea(key, w, h)` — Day lays the page out in it.
static napi_value NavPageArea(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double key = 0, w = 0, h = 0;
    napi_get_value_double(env, argv[0], &key);
    napi_get_value_double(env, argv[1], &w);
    napi_get_value_double(env, argv[2], &h);
    day_arkui_nav_area((uint64_t)key, w, h);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// Set a process environment variable from ArkTS: `setEnv(key, value)`. The launcher (`day launch`
// / hdc `aa start --ps`) hands the app its dayscript engine port + token (and locale / autodrive)
// this way, and the ArkTS EntryAbility applies them BEFORE `start()` runs `day_script::init()`.
// This is the HarmonyOS analogue of Android's intent-extra → setenv env delivery (day/src/lib.rs).
// `setenv` mutates the same `environ` Rust's `std::env::var` reads, so no Rust round-trip is needed.
static napi_value SetEnv(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string key = napi_to_string(env, argv[0]);
    std::string val = napi_to_string(env, argv[1]);
    if (!key.empty()) setenv(key.c_str(), val.c_str(), 1);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

static napi_value NapiInit(napi_env env, napi_value exports) {
    napi_value fn;
    napi_create_function(env, "start", NAPI_AUTO_LENGTH, DayStart, nullptr, &fn);
    napi_set_named_property(env, exports, "start", fn);
    napi_create_function(env, "setEnv", NAPI_AUTO_LENGTH, SetEnv, nullptr, &fn);
    napi_set_named_property(env, exports, "setEnv", fn);
    napi_create_function(env, "registerFilePicker", NAPI_AUTO_LENGTH, RegisterFilePicker, nullptr,
                         &fn);
    napi_set_named_property(env, exports, "registerFilePicker", fn);
    napi_create_function(env, "registerOpenUrl", NAPI_AUTO_LENGTH, RegisterOpenUrl, nullptr, &fn);
    napi_set_named_property(env, exports, "registerOpenUrl", fn);
    napi_create_function(env, "onFileResult", NAPI_AUTO_LENGTH, OnFileResult, nullptr, &fn);
    napi_set_named_property(env, exports, "onFileResult", fn);
    napi_create_function(env, "registerResourceManager", NAPI_AUTO_LENGTH, RegisterResourceManager,
                         nullptr, &fn);
    napi_set_named_property(env, exports, "registerResourceManager", fn);
    napi_create_function(env, "registerNav", NAPI_AUTO_LENGTH, RegisterNav, nullptr, &fn);
    napi_set_named_property(env, exports, "registerNav", fn);
    napi_create_function(env, "navPopped", NAPI_AUTO_LENGTH, NavPopped, nullptr, &fn);
    napi_set_named_property(env, exports, "navPopped", fn);
    napi_create_function(env, "navPageArea", NAPI_AUTO_LENGTH, NavPageArea, nullptr, &fn);
    napi_set_named_property(env, exports, "navPageArea", fn);
    return exports;
}

// The module name must match the imported `.so` basename. Day's HarmonyOS app cdylib is built as
// `libentry.so` (the DevEco convention; the crate uses `[lib] name = "entry"`), imported from ArkTS
// as `import native from 'libentry.so'`.
static napi_module g_day_module = {
    /* .nm_version =    */ 1,
    /* .nm_flags =      */ 0,
    /* .nm_filename =   */ nullptr,
    /* .nm_register_func= */ NapiInit,
    /* .nm_modname =    */ "entry",
    /* .nm_priv =       */ nullptr,
    /* .reserved =      */ {0},
};

extern "C" __attribute__((constructor)) void day_arkui_register_module(void) {
    napi_module_register(&g_day_module);
}
