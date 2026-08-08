// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-arkui-sys — a flat C ABI over the HarmonyOS ArkUI Native NodeAPI (arkui/native_node.h) and
// NAPI (napi/native_api.h), the HarmonyOS analogue of day-qt-sys / day-xaml-sys. day builds the
// widget tree natively (createNode/setAttribute/addChild) and mounts it into an ArkTS `NodeContent`
// slot; native events call back into Rust by node id; main-thread posting rides libuv (uv_async).
//
// The ArkUI headers assume C++ (bool, forward-declared types), so this is compiled as C++.

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <map>
#include <mutex>
#include <string>

#include <sys/mman.h>
#include <unistd.h>

#include <arkui/drag_and_drop.h>
#include <arkui/native_gesture.h>
#include <arkui/native_interface.h>
#include <arkui/native_interface_focus.h>
#include <arkui/native_node.h>
#include <arkui/native_node_napi.h>
#include <arkui/native_type.h>
#include <arkui/ui_input_event.h>
#include <hilog/log.h>
#include <napi/native_api.h>
#include <rawfile/raw_file.h>
#include <rawfile/raw_file_manager.h>
#include <uv.h>

// OH_Drawing (native 2-D) for the canvas custom node's on-draw callback (§11).
#include <native_drawing/drawing_brush.h>
#include <native_drawing/drawing_canvas.h>
#include <native_drawing/drawing_error_code.h>
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
// Deep-link intake (docs/deep-links.md): cold and warm links land here; the app cdylib
// buffers or navigates as appropriate.
extern "C" void day_arkui_deeplink(const char* uri);
extern "C" void day_arkui_on_event(uint64_t id, int32_t kind, double num, const char* text);

// Event kinds — mirror of day_spec::bridge::BridgeKind (the shared wire table; same numbers as
// the Android bridge). day-arkui-sys's bridge_kinds_parity test reads THIS block and asserts
// each value against the Rust enum — edit both together.
#define DAY_K_PRESSED 0
#define DAY_K_TEXT_CHANGED 1
#define DAY_K_TOGGLE_CHANGED 2
#define DAY_K_VALUE_CHANGED 3
#define DAY_K_SELECTION_CHANGED 4
#define DAY_K_GESTURE 11
#define DAY_K_CUSTOM 12
#define DAY_K_PRESENT_FILE 15
#define DAY_K_FOCUS_CHANGED 16
#define DAY_K_SUBMITTED 17
#define DAY_K_VALUE_COMMITTED 22
extern "C" void day_arkui_set_cache_dir(const char* path);
// Recycling-list callbacks into Rust (docs/list.md): row count, and build/rebind a row's content
// into the native cell (a plain Stack `cell`) — plus recycle when a cell scrolls out.
extern "C" uint32_t day_arkui_list_count(uint64_t host_id);
extern "C" void day_arkui_list_bind(uint64_t host_id, uint32_t index, void* cell);
// Drag-to-reorder (docs/list.md): the guard's verdict (accepted index or -1) and the commit,
// both synchronous Rust exports like day_arkui_list_count.
extern "C" int32_t day_arkui_list_can_move(uint64_t host_id, uint32_t from, uint32_t to);
extern "C" uint32_t day_arkui_list_move(uint64_t host_id, uint32_t from, uint32_t to);

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
static napi_ref g_nav_set_guard = nullptr; // (on: boolean) => void — arm the top-page back guard
static napi_ref g_nav_menu = nullptr; // (icon: string, label: string, action: number) => void —
                                      // set the trailing title-bar action (NavProps::bar_action)
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
extern "C" void day_arkui_nav_back_requested();
extern "C" void day_arkui_nav_area(uint64_t key, double w, double h);
extern "C" void day_arkui_nav_menu_action(uint64_t action);
extern "C" void day_arkui_resized(double w, double h);

// ---- ArkTS-built piece components (docs/extending.md) -----------------------
// Some native components exist ONLY in ArkTS: the declarative `Web` has no ArkUI C-API node kind
// (native_node.h stops at the container types), and neither does `Map`. A piece that wraps one
// ships its own .ets (staged into the hvigor project by `day build`) and registers a factory here.
// `make` builds the component in a BuilderNode and returns its FrameNode, which
// OH_ArkUI_GetNodeHandleFromNapiValue turns into an ArkUI_NodeHandle Day mounts like any other
// node. `props`/`cmd`/`arg` are opaque strings — the piece owns both ends, so the bridge never
// grows a case per piece. Unregistered is a safe no-op (the kind falls back to a placeholder).
static napi_ref g_piece_make = nullptr;    // (kind: string, id: number, props: string) => FrameNode
static napi_ref g_piece_update = nullptr;  // (id: number, cmd: string, arg: string) => void
static napi_ref g_piece_dispose = nullptr; // (id: number) => void — release the BuilderNode

// ---- main-thread posting (uv_async on the JS event loop) -------------------
struct PostItem {
    void (*cb)(void*);
    void* data;
};
static uv_async_t g_async;
static std::mutex g_mtx;
static std::deque<PostItem> g_queue;
static bool g_async_ready = false;
// The JS event loop (captured at start) — day_ark_post_delayed's uv_timer needs it.
static uv_loop_t* g_loop = nullptr;

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
// List drag-to-reorder handlers (docs/list.md), defined with the list state below.
static void day_list_on_drag_start(ArkUI_NodeEvent* ev);
static void day_list_on_drop(ArkUI_NodeEvent* ev);

static void event_receiver(ArkUI_NodeEvent* ev) {
    if (!ev) return;
    uint64_t id = (uint64_t)(uintptr_t)OH_ArkUI_NodeEvent_GetUserData(ev);
    ArkUI_NodeEventType t = OH_ArkUI_NodeEvent_GetEventType(ev);
    switch (t) {
        case NODE_ON_CLICK:
            day_arkui_on_event(id, DAY_K_PRESSED, 0.0, "");
            break;
        // Drag-to-reorder (docs/list.md) — the list state lives later in this file, so the
        // arms defer to forward-declared handlers.
        case NODE_ON_DRAG_START:
            day_list_on_drag_start(ev);
            break;
        case NODE_ON_DROP:
            day_list_on_drop(ev);
            break;
        case NODE_TEXT_INPUT_ON_CHANGE: {
            auto* s = OH_ArkUI_NodeEvent_GetStringAsyncEvent(ev);
            day_arkui_on_event(id, DAY_K_TEXT_CHANGED, 0.0, (s && s->pStr) ? s->pStr : "");
            break;
        }
        case NODE_TOGGLE_ON_CHANGE: {
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, DAY_K_TOGGLE_CHANGED, c ? (double)c->data[0].i32 : 0.0, "");
            break;
        }
        case NODE_SLIDER_EVENT_ON_CHANGE: {
            // data[0].f32 is the value; data[1].i32 is the state that triggered the event —
            // ArkTS's SliderChangeMode (Begin 0, Moving 1, End 2, Click 3). ArkUI is the one
            // toolkit that hands the phase over directly, so the settled value needs no
            // tracking flag: End ends a drag, Click is a jump to a point on the track and is
            // already settled. The enum has no C name in the SDK headers, hence the literals.
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            double value = c ? (double)c->data[0].f32 : 0.0;
            int mode = c ? c->data[1].i32 : 1;
            day_arkui_on_event(id, DAY_K_VALUE_CHANGED, value, "");
            if (mode == 2 || mode == 3) {
                day_arkui_on_event(id, DAY_K_VALUE_COMMITTED, value, "");
            }
            break;
        }
        case NODE_SWIPER_EVENT_ON_CHANGE: {
            // The active page index (SelectionChanged).
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, DAY_K_SELECTION_CHANGED, c ? (double)c->data[0].i32 : 0.0, "");
            break;
        }
        case NODE_TEXT_AREA_ON_CHANGE: {
            auto* s = OH_ArkUI_NodeEvent_GetStringAsyncEvent(ev);
            day_arkui_on_event(id, DAY_K_TEXT_CHANGED, 0.0, (s && s->pStr) ? s->pStr : "");
            break;
        }
        case NODE_TEXT_PICKER_EVENT_ON_CHANGE: {
            // The selected option index (SelectionChanged).
            auto* c = OH_ArkUI_NodeEvent_GetNodeComponentEvent(ev);
            day_arkui_on_event(id, DAY_K_SELECTION_CHANGED, c ? (double)c->data[0].f32 : 0.0, "");
            break;
        }
        // Focus pair + text-input submit (docs/focus.md) — kinds match the Android bridge.
        case NODE_ON_FOCUS:
            day_arkui_on_event(id, DAY_K_FOCUS_CHANGED, 1.0, "");
            break;
        case NODE_ON_BLUR:
            day_arkui_on_event(id, DAY_K_FOCUS_CHANGED, 0.0, "");
            break;
        case NODE_TEXT_INPUT_ON_SUBMIT:
            day_arkui_on_event(id, DAY_K_SUBMITTED, 0.0, "");
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
        case 16: return ARKUI_NODE_TEXT_AREA;   // multi-line editor (docs/textarea.md)
        case 17: return ARKUI_NODE_TEXT_PICKER; // native option wheel (docs/picker.md)
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
// Minimal scroll so [x,y,w,h] (content vp) is visible — scrollRectToVisible semantics.
// ArkUI positions by absolute offset (NODE_SCROLL_OFFSET, get/set), so read the current
// offset + the node's own size, compute the reveal, and write the offset back (clamped by
// the node to its scrollable range).
void day_ark_scroll_to_rect(void* n, float x, float y, float w, float h, int animated) {
    if (!g_api || !n) return;
    float ox = 0.0f, oy = 0.0f;
    const ArkUI_AttributeItem* cur = g_api->getAttribute((ArkUI_NodeHandle)n, NODE_SCROLL_OFFSET);
    if (cur && cur->size >= 2) {
        ox = cur->value[0].f32;
        oy = cur->value[1].f32;
    }
    float pw = 0.0f, ph = 0.0f;
    const ArkUI_AttributeItem* wa = g_api->getAttribute((ArkUI_NodeHandle)n, NODE_WIDTH);
    const ArkUI_AttributeItem* ha = g_api->getAttribute((ArkUI_NodeHandle)n, NODE_HEIGHT);
    if (wa && wa->size >= 1) pw = wa->value[0].f32;
    if (ha && ha->size >= 1) ph = ha->value[0].f32;
    float nx = ox, ny = oy;
    if (x + w > nx + pw) nx = x + w - pw;
    if (x < nx) nx = x;
    if (y + h > ny + ph) ny = y + h - ph;
    if (y < ny) ny = y;
    ArkUI_NumberValue nv[3];
    nv[0].f32 = nx;
    nv[1].f32 = ny;
    nv[2].i32 = animated ? 300 : 0; // animation duration ms (0 = jump)
    ArkUI_AttributeItem it{};
    it.value = nv;
    it.size = 3;
    int rc = g_api->setAttribute((ArkUI_NodeHandle)n, NODE_SCROLL_OFFSET, &it);
    OH_LOG_Print(LOG_APP, LOG_WARN, 0xDA11, "day",
                 "scroll_to_rect target=(%{public}.1f,%{public}.1f %{public}.1fx%{public}.1f) "
                 "cur=(%{public}.1f,%{public}.1f) view=%{public}.1fx%{public}.1f -> "
                 "(%{public}.1f,%{public}.1f) rc=%{public}d",
                 x, y, w, h, ox, oy, pw, ph, nx, ny, rc);
}
void day_ark_insert_child(void* p, void* c, int32_t pos) {
    if (g_api) g_api->insertChildAt((ArkUI_NodeHandle)p, (ArkUI_NodeHandle)c, pos);
}
void day_ark_remove_child(void* p, void* c) {
    if (g_api) g_api->removeChild((ArkUI_NodeHandle)p, (ArkUI_NodeHandle)c);
}

void day_ark_set_text(void* n, const char* s) { set_str(n, NODE_TEXT_CONTENT, s); }
// Make a Text node's text user-selectable + copyable (the `.selectable()` modifier, docs/text.md).
// A non-Text node ignores NODE_TEXT_COPY_OPTION (setAttribute returns an error, no crash).
void day_ark_label_set_selectable(void* n, int on) {
    if (!g_api || !n) return;
    ArkUI_NumberValue nv;
    nv.i32 = on ? ARKUI_COPY_OPTIONS_LOCAL_DEVICE : ARKUI_COPY_OPTIONS_NONE;
    ArkUI_AttributeItem it{};
    it.value = &nv;
    it.size = 1;
    g_api->setAttribute((ArkUI_NodeHandle)n, NODE_TEXT_COPY_OPTION, &it);
}
void day_ark_set_button_label(void* n, const char* s) { set_str(n, NODE_BUTTON_LABEL, s); }
void day_ark_set_input_text(void* n, const char* s) { set_str(n, NODE_TEXT_INPUT_TEXT, s); }
void day_ark_set_placeholder(void* n, const char* s) { set_str(n, NODE_TEXT_INPUT_PLACEHOLDER, s); }

// Text area (docs/textarea.md): seed/replace text + placeholder.
void day_ark_set_textarea_text(void* n, const char* s) { set_str(n, NODE_TEXT_AREA_TEXT, s); }
void day_ark_set_textarea_placeholder(void* n, const char* s) {
    set_str(n, NODE_TEXT_AREA_PLACEHOLDER, s);
}

// Picker (docs/picker.md): the option range is a ';'-joined string per the ArkUI C API, and the
// selected index is a u32 attribute. HarmonyOS has no segmented control, so every day picker
// style maps to the native TEXT_PICKER wheel.
void day_ark_set_picker(void* n, const char* options_semi, uint32_t selected) {
    set_str(n, NODE_TEXT_PICKER_OPTION_RANGE, options_semi);
    set_u32(n, NODE_TEXT_PICKER_OPTION_SELECTED, selected);
}
void day_ark_set_picker_selected(void* n, uint32_t selected) {
    set_u32(n, NODE_TEXT_PICKER_OPTION_SELECTED, selected);
}
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
// SVG-only recolor: NODE_IMAGE_FILL_COLOR repaints every path of an SVG src with `argb`
// (raster sources ignore it) — how nav-row vector icons tint (docs/vectors.md).
void day_ark_set_image_fill(void* n, uint32_t argb) { set_u32(n, NODE_IMAGE_FILL_COLOR, argb); }
// Whether rawfile `path` (e.g. "day/home.svg") exists in the app package; 0 before the entry
// ability registers the resource manager. Lets day-arkui prefer a vector's staged SVG over
// the raster fallback under the same stem (docs/vectors.md).
int32_t day_ark_rawfile_exists(const char* path) {
    if (!g_res_mgr) return 0;
    RawFile* f = OH_ResourceManager_OpenRawFile(g_res_mgr, path);
    if (!f) return 0;
    OH_ResourceManager_CloseRawFile(f);
    return 1;
}
// One margin (vp) on all four sides — the nav rows gap icon from label with it (symmetric,
// so RTL row direction needs no flip).
void day_ark_set_margin(void* n, double vp) { set_f32(n, NODE_MARGIN, (float)vp); }
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
// platform/harmony scaffold's EntryAbility (ArkTS font.registerFont) before the native UI loads;
// ArkUI falls back to the default family when the name isn't registered.
void day_ark_set_font_family(void* n, const char* family) { set_str(n, NODE_FONT_FAMILY, family); }
// Tabular figures through the OpenType feature string ArkUI's NODE_FONT_FEATURE takes, so the
// registered face is untouched and only the digits change metrics. A font without `tnum`, or an
// SDK predating the attribute, ignores it — the documented degradation.
void day_ark_set_font_feature(void* n, const char* feature) {
    // No #ifdef around this: NODE_FONT_FEATURE is an ENUMERATOR of ArkUI_NodeAttributeType
    // (native_node.h), not a preprocessor macro, so `#ifdef NODE_FONT_FEATURE` is false on every
    // SDK — including the ones that have it — and quietly compiled the feature away. day builds
    // against API 18 (build-profile.json5's compatibleSdkVersion), where the attribute is present.
    set_str(n, NODE_FONT_FEATURE, feature);
}
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
        case 7: t = NODE_TEXT_AREA_ON_CHANGE; break;
        case 8: t = NODE_TEXT_PICKER_EVENT_ON_CHANGE; break;
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

// Delayed main-thread post (the frame clock's tick source, §8.4): run `cb(data)` after `ms`
// on the JS loop via a one-shot uv_timer. JS thread only (uv_timer_init is not thread-safe).
struct DayTimer {
    uv_timer_t timer; // first member: the uv_handle_t* IS the DayTimer*
    void (*cb)(void*);
    void* data;
};
static void day_timer_closed(uv_handle_t* h) { free(h); }
static void day_timer_fire(uv_timer_t* t) {
    DayTimer* dt = (DayTimer*)t;
    void (*cb)(void*) = dt->cb;
    void* data = dt->data;
    uv_timer_stop(t);
    uv_close((uv_handle_t*)t, day_timer_closed);
    cb(data);
}
void day_ark_post_delayed(void (*cb)(void*), void* data, uint32_t ms) {
    if (!g_loop) {
        day_ark_post(cb, data);
        return;
    }
    DayTimer* dt = (DayTimer*)malloc(sizeof(DayTimer));
    if (!dt) return;
    dt->cb = cb;
    dt->data = data;
    uv_timer_init(g_loop, &dt->timer);
    uv_timer_start(&dt->timer, day_timer_fire, ms, 0);
}

// Pan/drag gesture (docs/shapes.md): a native pan recognizer whose events reach Rust as the
// shared kind-11 gesture wire ("x,y,tx,ty" in px; phase 1=began 2=changed 3=ended). Location
// is component-relative from the raw pointer event.
static ArkUI_NativeGestureAPI_1* g_gesture = nullptr;
static void pan_receiver(ArkUI_GestureEvent* ev, void* extra) {
    uint64_t id = (uint64_t)(uintptr_t)extra;
    ArkUI_GestureEventActionType action = OH_ArkUI_GestureEvent_GetActionType(ev);
    double phase;
    switch (action) {
        case GESTURE_EVENT_ACTION_ACCEPT: phase = 1.0; break;
        case GESTURE_EVENT_ACTION_UPDATE: phase = 2.0; break;
        default: phase = 3.0; break; // END or CANCEL
    }
    float tx = OH_ArkUI_PanGesture_GetOffsetX(ev);
    float ty = OH_ArkUI_PanGesture_GetOffsetY(ev);
    float x = 0.0f, y = 0.0f;
    const ArkUI_UIInputEvent* in = OH_ArkUI_GestureEvent_GetRawInputEvent(ev);
    if (in) {
        x = OH_ArkUI_PointerEvent_GetX(in);
        y = OH_ArkUI_PointerEvent_GetY(in);
    }
    char buf[96];
    snprintf(buf, sizeof buf, "%.2f,%.2f,%.2f,%.2f", x, y, tx, ty);
    day_arkui_on_event(id, DAY_K_GESTURE, phase, buf);
}
void day_ark_enable_pan(void* node, uint64_t id) {
    if (!g_gesture) {
        OH_ArkUI_GetModuleInterface(ARKUI_NATIVE_GESTURE, ArkUI_NativeGestureAPI_1, g_gesture);
    }
    if (!g_gesture || !node) return;
    ArkUI_GestureRecognizer* pan =
        g_gesture->createPanGesture(1, GESTURE_DIRECTION_ALL, 3.0);
    if (!pan) return;
    g_gesture->setGestureEventTarget(
        pan,
        (ArkUI_GestureEventActionTypeMask)(GESTURE_EVENT_ACTION_ACCEPT |
                                           GESTURE_EVENT_ACTION_UPDATE |
                                           GESTURE_EVENT_ACTION_END |
                                           GESTURE_EVENT_ACTION_CANCEL),
        (void*)(uintptr_t)id, pan_receiver);
    g_gesture->addGestureToNode((ArkUI_NodeHandle)node, pan, NORMAL, NORMAL_GESTURE_MASK);
}

double day_ark_density(void) { return g_density; }

// Ask the ArkTS-registered picker to open/save a file. Runs on the JS thread, so a plain
// napi_call_function is safe. Falls back to an immediate cancel if nothing is registered.
void day_ark_present_file(uint64_t req, int32_t mode, const char* name, const char* src,
                          const char* filters) {
    if (!g_env || !g_file_picker) {
        day_arkui_on_event(req, DAY_K_PRESENT_FILE, 0.0, ""); // cancel (no picker)
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
        day_arkui_on_event(req, DAY_K_PRESENT_FILE, 0.0, "");
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
// Arm/disarm the top NavDestination's back guard (NavPatch::GuardTop). No-op if the ArkTS host
// predates the seam (g_nav_set_guard null) — the app simply gets no ArkUI guard, back works.
void day_ark_nav_set_guard(int on) {
    if (!g_env || !g_nav_set_guard) return;
    napi_value cb = nullptr, self = nullptr, arg = nullptr, ret = nullptr;
    napi_get_reference_value(g_env, g_nav_set_guard, &cb);
    napi_get_undefined(g_env, &self);
    napi_get_boolean(g_env, on != 0, &arg);
    napi_call_function(g_env, self, cb, 1, &arg, &ret);
}

// Set the trailing title-bar action (NavProps::bar_action, docs/navigation.md): the ArkTS side
// stores it and renders it as a `.menus()` item on every NavDestination. No-op if the ArkTS host
// predates the seam (g_nav_menu null) — the app simply gets no bar action. JS thread only.
void day_ark_nav_set_menu(const char* icon, const char* label, uint64_t action) {
    if (!g_env || !g_nav_menu) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_nav_menu, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value args[3];
        napi_create_string_utf8(g_env, icon ? icon : "", NAPI_AUTO_LENGTH, &args[0]);
        napi_create_string_utf8(g_env, label ? label : "", NAPI_AUTO_LENGTH, &args[1]);
        napi_create_double(g_env, (double)action, &args[2]);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 3, args, &ret);
    }
    napi_close_handle_scope(g_env, scope);
}

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
                float x = a, y = b;
                if (f == 1.0f) {
                    // Centered on (a,b): measured width, and the baseline placed from the
                    // font metrics (Skia-style: ascent negative, descent positive) — the
                    // same formula as the Android canvas backend, so glyphs land dead
                    // center on every platform (the 2048 tile digits are the acid test).
                    float w = 0.0f;
                    if (OH_Drawing_FontMeasureText(font, s.c_str(), s.size(),
                                                   TEXT_ENCODING_UTF8, nullptr,
                                                   &w) == OH_DRAWING_SUCCESS) {
                        x = a - w / 2.0f;
                    } else {
                        x = a - (float)s.size() * e * 0.28f; // fallback guess
                    }
                    OH_Drawing_Font_Metrics m;
                    OH_Drawing_FontGetMetrics(font, &m);
                    y = b - (m.ascent + m.descent) / 2.0f;
                }
                OH_Drawing_CanvasDrawTextBlob(cv, blob, x, y);
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
    // Drag-to-reorder (docs/list.md): whether rows drag, the list node (for geometry), and each
    // live cell's currently-bound row (cells recycle, so the row is re-recorded on every bind).
    bool reorderable;
    ArkUI_NodeHandle list_node;
    std::map<ArkUI_NodeHandle, int> rows;
};
static std::map<void*, DayList*> g_lists; // list node → its adapter binding

// The in-flight reorder drag (one at a time; day lists accept only their OWN rows).
static DayList* g_drag_list = nullptr;
static int g_drag_from = -1;

// The slot under a drag event's touch point, in the list's row grid.
static int day_list_drag_slot(DayList* dl, ArkUI_DragEvent* de) {
    if (!dl || dl->row_h <= 0) return -1;
    ArkUI_IntOffset pos{};
    OH_ArkUI_NodeUtils_GetLayoutPositionInWindow(dl->list_node, &pos);
    float y = OH_ArkUI_DragEvent_GetTouchPointYToWindow(de) - (float)pos.y;
    // Add the list's scroll offset (vp → px) so the slot is content-absolute.
    const ArkUI_AttributeItem* got = g_api->getAttribute(dl->list_node, NODE_SCROLL_OFFSET);
    if (got && got->size >= 2) y += got->value[1].f32 * g_density;
    int n = (int)OH_ArkUI_NodeAdapter_GetTotalNodeCount(dl->adapter);
    if (n <= 0) return -1;
    int slot = (int)(y / dl->row_h);
    return slot < 0 ? 0 : (slot >= n ? n - 1 : slot);
}

// A row lifted (docs/list.md): the node is a pooled LIST_ITEM — find its list + currently-bound
// row (the maps are small: one entry per live cell).
static void day_list_on_drag_start(ArkUI_NodeEvent* ev) {
    ArkUI_NodeHandle n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
    for (auto& entry : g_lists) {
        DayList* dl = entry.second;
        auto it = dl->rows.find(n);
        if (it != dl->rows.end()) {
            g_drag_list = dl;
            g_drag_from = it->second;
            break;
        }
    }
}

// A drop over the list node: vet through the app's guard and commit through the sync seam; a
// denied drop reports FAILED so ArkUI's native spring-back carries the affordance.
static void day_list_on_drop(ArkUI_NodeEvent* ev) {
    ArkUI_NodeHandle n = OH_ArkUI_NodeEvent_GetNodeHandle(ev);
    ArkUI_DragEvent* de = OH_ArkUI_NodeEvent_GetDragEvent(ev);
    auto found = g_lists.find((void*)n);
    DayList* dl = (found != g_lists.end()) ? found->second : nullptr;
    if (!de || !dl || dl != g_drag_list || g_drag_from < 0) return;
    int from = g_drag_from;
    g_drag_list = nullptr;
    g_drag_from = -1;
    int slot = day_list_drag_slot(dl, de);
    int accepted = slot < 0 ? -1 : day_arkui_list_can_move(dl->host_id, from, slot);
    if (accepted >= 0) {
        if (accepted != from) {
            day_arkui_list_move(dl->host_id, (uint32_t)from, (uint32_t)accepted);
            OH_ArkUI_NodeAdapter_ReloadAllItems(dl->adapter);
        }
        OH_ArkUI_DragEvent_SetDragResult(de, ARKUI_DRAG_RESULT_SUCCESSFUL);
    } else {
        OH_ArkUI_DragEvent_SetDragResult(de, ARKUI_DRAG_RESULT_FAILED);
    }
}

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
                if (dl->reorderable) {
                    // Long-press lifts the row with ArkUI's own drag preview; the drop lands on
                    // the LIST node's NODE_ON_DROP (registered in day_ark_list_init).
                    OH_ArkUI_SetNodeDraggable(cell, true);
                    g_api->registerNodeEvent(cell, NODE_ON_DRAG_START, 0, nullptr);
                }
            }
            if (dl->reorderable) dl->rows[cell] = (int)idx;
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

void day_ark_list_init(void* node, uint64_t host_id, double row_h_vp, uint32_t reorderable) {
    if (!g_api || !node) return;
    DayList* dl = new DayList{OH_ArkUI_NodeAdapter_Create(),
                              host_id,
                              (float)(row_h_vp * g_density),
                              {},
                              reorderable != 0,
                              (ArkUI_NodeHandle)node,
                              {}};
    OH_ArkUI_NodeAdapter_RegisterEventReceiver(dl->adapter, dl, list_adapter_receiver);
    g_lists[node] = dl;
    ArkUI_AttributeItem it{};
    it.object = dl->adapter;
    g_api->setAttribute((ArkUI_NodeHandle)node, NODE_LIST_NODE_ADAPTER, &it);
    if (dl->reorderable) {
        // Accept day-row drops anywhere over the list; the verdict comes from the app's guard
        // at drop time (day_arkui_list_can_move).
        OH_ArkUI_AllowNodeAllDropDataTypes((ArkUI_NodeHandle)node);
        g_api->registerNodeEvent((ArkUI_NodeHandle)node, NODE_ON_DROP, 0, nullptr);
    }
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

// Scroll the list so row `index` is visible (docs/list.md), realizing it if needed.
void day_ark_list_scroll_to_row(void* node, uint32_t index) {
    auto it = g_lists.find(node);
    if (it == g_lists.end()) return;
    int32_t n = (int32_t)OH_ArkUI_NodeAdapter_GetTotalNodeCount(it->second->adapter);
    if (n <= 0) return;
    ArkUI_NumberValue v[1];
    v[0].i32 = (int32_t)index < n ? (int32_t)index : n - 1;
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
    g_loop = loop;

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

// `deepLink(uri)` — a cold `want.uri` or a warm `onNewWant` one (docs/deep-links.md). Safe
// to call before `start`: the app side buffers until the first mount.
static napi_value DayDeepLink(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    napi_value undef;
    napi_get_undefined(env, &undef);
    if (argc < 1) return undef;
    size_t len = 0;
    if (napi_get_value_string_utf8(env, argv[0], nullptr, 0, &len) != napi_ok || len == 0)
        return undef;
    std::string uri(len, '\0');
    napi_get_value_string_utf8(env, argv[0], uri.data(), len + 1, &len);
    day_arkui_deeplink(uri.c_str());
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

// ---- secondary windows (docs/windows.md) -----------------------------------
// ArkTS registers the multiton-ability launchers: `registerWindows(open, close)` where
// `open` = `(node: number, title: string) => void` (startAbility on DayWindowAbility with
// the node/title as want parameters) and `close` = `(node: number) => void`
// (terminateSelf on that instance's context). The window ability's page completes the
// open through `windowStart(nodeContent, node, w, h)`, and reports its lifecycle through
// `windowClosed(node)` / `windowFocused(node, active)`.
static napi_ref g_open_window = nullptr;
static napi_ref g_close_window = nullptr;

static napi_value RegisterWindows(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    if (g_open_window) { napi_delete_reference(env, g_open_window); g_open_window = nullptr; }
    if (g_close_window) { napi_delete_reference(env, g_close_window); g_close_window = nullptr; }
    if (argc > 0 && argv[0]) napi_create_reference(env, argv[0], 1, &g_open_window);
    if (argc > 1 && argv[1]) napi_create_reference(env, argv[1], 1, &g_close_window);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// Rust-facing diagnostic logging: Rust stderr never reaches hilog, so day-arkui routes
// its framework diagnostics through the app log channel here.
extern "C" void day_ark_log(const char* msg) {
    OH_LOG_Print(LOG_APP, LOG_WARN, 0xDA11, "day", "%{public}s", msg ? msg : "");
}

// Rust-facing: whether the ArkTS host registered the window launchers (drives
// Cap::MultiWindow — an older host degrades to the cover fallback gracefully).
extern "C" int day_ark_has_windows(void) { return g_env && g_open_window ? 1 : 0; }

// Rust-facing: launch a secondary day window. 1 = the request went out (the ability's
// page completes the open); 0 = no launcher registered.
extern "C" int day_ark_open_window(unsigned long long node, const char* title) {
    if (!g_env || !g_open_window) return 0;
    napi_value cb = nullptr, undef = nullptr;
    napi_get_reference_value(g_env, g_open_window, &cb);
    if (!cb) return 0;
    napi_get_undefined(g_env, &undef);
    napi_value args[2];
    napi_create_double(g_env, static_cast<double>(node), &args[0]);
    napi_create_string_utf8(g_env, title ? title : "", NAPI_AUTO_LENGTH, &args[1]);
    napi_value ignored = nullptr;
    napi_call_function(g_env, undef, cb, 2, args, &ignored);
    return 1;
}

// Rust-facing: close a secondary window's ability instance.
extern "C" void day_ark_close_window(unsigned long long node) {
    if (!g_env || !g_close_window) return;
    napi_value cb = nullptr, undef = nullptr;
    napi_get_reference_value(g_env, g_close_window, &cb);
    if (!cb) return;
    napi_get_undefined(g_env, &undef);
    napi_value arg;
    napi_create_double(g_env, static_cast<double>(node), &arg);
    napi_value ignored = nullptr;
    napi_call_function(g_env, undef, cb, 1, &arg, &ignored);
}

extern "C" int day_arkui_window_start(unsigned long long node, void* content,
                                      double w_vp, double h_vp);
extern "C" void day_arkui_window_resized(unsigned long long node, double w_vp, double h_vp);
extern "C" void day_arkui_window_closed(unsigned long long node);
extern "C" void day_arkui_window_focused(unsigned long long node, int active);

// The window ability's page hands its NodeContent + day node id here: `windowStart(
// nodeContent, node, w, h)` → true when the pending open completed (false = closed before
// connecting; the page's ability terminates itself).
static napi_value DayWindowStart(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    ArkUI_NodeContentHandle content = nullptr;
    OH_ArkUI_GetNodeContentFromNapiValue(env, argv[0], &content);
    double node = 0, w = 0, h = 0;
    napi_get_value_double(env, argv[1], &node);
    napi_get_value_double(env, argv[2], &w);
    napi_get_value_double(env, argv[3], &h);
    int ok = day_arkui_window_start(static_cast<unsigned long long>(node), content, w, h);
    napi_value out;
    napi_get_boolean(env, ok != 0, &out);
    return out;
}

// `windowResized(node, w, h)` — the secondary window's content area changed (vp).
static napi_value DayWindowResized(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double node = 0, w = 0, h = 0;
    napi_get_value_double(env, argv[0], &node);
    napi_get_value_double(env, argv[1], &w);
    napi_get_value_double(env, argv[2], &h);
    day_arkui_window_resized(static_cast<unsigned long long>(node), w, h);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// `windowClosed(node)` — the ability instance is going away (back, swipe, terminateSelf).
static napi_value DayWindowClosed(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double node = 0;
    napi_get_value_double(env, argv[0], &node);
    day_arkui_window_closed(static_cast<unsigned long long>(node));
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// `windowFocused(node, active)` — foreground/background transitions.
static napi_value DayWindowFocused(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double node = 0, active = 0;
    napi_get_value_double(env, argv[0], &node);
    napi_get_value_double(env, argv[1], &active);
    day_arkui_window_focused(static_cast<unsigned long long>(node), active != 0 ? 1 : 0);
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
// app hasn't registered one. `extern "C"` because this sits outside the extern "C" block above and
// Rust imports it unmangled — without it the symbol name-mangles and libentry.so fails to load.
extern "C" void day_ark_open_url(const char* url) {
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

// ---- ArkTS-built piece components (docs/extending.md) -----------------------
// ArkTS registers the piece factory + command sink + disposer, once, before `start()`:
// `registerPiece(make, update, dispose)`. `day build` generates the aggregator that calls this
// from every staged piece .ets, so an app with no such piece never registers and every entry
// point below stays a no-op.
static napi_value RegisterPiece(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    napi_ref* slots[3] = {&g_piece_make, &g_piece_update, &g_piece_dispose};
    for (size_t i = 0; i < 3; i++) {
        if (*slots[i]) {
            napi_delete_reference(env, *slots[i]);
            *slots[i] = nullptr;
        }
        if (i < argc && argv[i]) napi_create_reference(env, argv[i], 1, slots[i]);
    }
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// An ArkTS-built component reports back to its piece: `pieceEvent(id, text)`. Rides the SAME
// Custom channel the Android bridge uses (BridgeKind::Custom) — the payload is the whole event,
// so a piece that emits one kind of message needs no tag. JS thread only.
static napi_value PieceEvent(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double id = 0;
    napi_get_value_double(env, argv[0], &id);
    std::string text = napi_to_string(env, argv[1]);
    day_arkui_on_event((uint64_t)id, DAY_K_CUSTOM, 0.0, text.c_str());
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// Build a piece's ArkTS component and return its FrameNode as an ArkUI_NodeHandle Day can mount.
// Null when nothing is registered or the factory declined the kind — the caller then falls back to
// Day's placeholder leaf. JS thread only. `extern "C"` for the same reason as day_ark_open_url:
// this sits outside the extern "C" block above and Rust imports the symbol unmangled.
extern "C" void* day_ark_piece_make(const char* kind, uint64_t id, const char* props) {
    if (!g_env || !g_piece_make) return nullptr;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    void* out = nullptr;
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_piece_make, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value args[3];
        napi_create_string_utf8(g_env, kind ? kind : "", NAPI_AUTO_LENGTH, &args[0]);
        napi_create_double(g_env, (double)id, &args[1]);
        napi_create_string_utf8(g_env, props ? props : "", NAPI_AUTO_LENGTH, &args[2]);
        napi_value ret = nullptr;
        if (napi_call_function(g_env, undef, cb, 3, args, &ret) == napi_ok && ret) {
            ArkUI_NodeHandle node = nullptr;
            // Returns non-zero for undefined/null (an ArkTS factory that declined the kind),
            // leaving `node` untouched — hence the explicit init above.
            OH_ArkUI_GetNodeHandleFromNapiValue(g_env, ret, &node);
            out = (void*)node;
        }
    }
    napi_close_handle_scope(g_env, scope);
    return out;
}

// Send a piece command to its ArkTS side (the webview's load/back/forward/stop/reload). JS thread
// only; no-op when unregistered.
extern "C" void day_ark_piece_update(uint64_t id, const char* cmd, const char* arg) {
    if (!g_env || !g_piece_update) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_piece_update, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value args[3];
        napi_create_double(g_env, (double)id, &args[0]);
        napi_create_string_utf8(g_env, cmd ? cmd : "", NAPI_AUTO_LENGTH, &args[1]);
        napi_create_string_utf8(g_env, arg ? arg : "", NAPI_AUTO_LENGTH, &args[2]);
        napi_value ret;
        napi_call_function(g_env, undef, cb, 3, args, &ret);
    }
    napi_close_handle_scope(g_env, scope);
}

// Release the ArkTS BuilderNode behind a piece node. Called when Day disposes the native node —
// an ArkTS-owned component (a Web engine instance especially) outlives its FrameNode otherwise.
extern "C" void day_ark_piece_dispose(uint64_t id) {
    if (!g_env || !g_piece_dispose) return;
    napi_handle_scope scope;
    napi_open_handle_scope(g_env, &scope);
    napi_value cb = nullptr;
    napi_get_reference_value(g_env, g_piece_dispose, &cb);
    if (cb) {
        napi_value undef;
        napi_get_undefined(g_env, &undef);
        napi_value arg;
        napi_create_double(g_env, (double)id, &arg);
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
    day_arkui_on_event((uint64_t)reqd, DAY_K_PRESENT_FILE, 0.0, path.c_str());
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// ArkTS registers its Navigation bridge: `registerNav(push, pop, setTitle)` — see the
// Navigation-bridge comment at the top. Re-registration replaces the callbacks.
static napi_value RegisterNav(napi_env env, napi_callback_info info) {
    size_t argc = 5;
    napi_value argv[5] = {nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    g_env = env;
    napi_ref* refs[5] = {&g_nav_push, &g_nav_pop, &g_nav_title, &g_nav_set_guard, &g_nav_menu};
    for (size_t i = 0; i < 5; i++) {
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

// The trailing title-bar action was tapped (NavProps::bar_action): dispatch its registered
// closure by id (docs/navigation.md). `navMenuAction(action)`.
static napi_value NavMenuAction(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double action = 0;
    napi_get_value_double(env, argv[0], &action);
    day_arkui_nav_menu_action((uint64_t)action);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

// A guarded NavDestination's back was pressed (onBackPressed): the ArkTS side consumed the
// native pop and asks Rust's guard to decide (docs/navigation.md).
static napi_value NavBackRequested(napi_env env, napi_callback_info info) {
    (void)info;
    day_arkui_nav_back_requested();
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

// Root-area change after start (keyboard RESIZE avoidance, rotation): `resized(w, h)` in vp.
static napi_value DayResized(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double w = 0, h = 0;
    napi_get_value_double(env, argv[0], &w);
    napi_get_value_double(env, argv[1], &h);
    day_arkui_resized(w, h);
    napi_value undef;
    napi_get_undefined(env, &undef);
    return undef;
}

static napi_value NapiInit(napi_env env, napi_value exports) {
    napi_value fn;
    napi_create_function(env, "start", NAPI_AUTO_LENGTH, DayStart, nullptr, &fn);
    napi_set_named_property(env, exports, "start", fn);
    napi_create_function(env, "resized", NAPI_AUTO_LENGTH, DayResized, nullptr, &fn);
    napi_set_named_property(env, exports, "resized", fn);
    napi_create_function(env, "setEnv", NAPI_AUTO_LENGTH, SetEnv, nullptr, &fn);
    napi_set_named_property(env, exports, "setEnv", fn);
    napi_create_function(env, "registerFilePicker", NAPI_AUTO_LENGTH, RegisterFilePicker, nullptr,
                         &fn);
    napi_set_named_property(env, exports, "registerFilePicker", fn);
    napi_create_function(env, "registerOpenUrl", NAPI_AUTO_LENGTH, RegisterOpenUrl, nullptr, &fn);
    napi_set_named_property(env, exports, "registerOpenUrl", fn);
    napi_create_function(env, "registerWindows", NAPI_AUTO_LENGTH, RegisterWindows, nullptr, &fn);
    napi_set_named_property(env, exports, "registerWindows", fn);
    napi_create_function(env, "windowStart", NAPI_AUTO_LENGTH, DayWindowStart, nullptr, &fn);
    napi_set_named_property(env, exports, "windowStart", fn);
    napi_create_function(env, "windowResized", NAPI_AUTO_LENGTH, DayWindowResized, nullptr, &fn);
    napi_set_named_property(env, exports, "windowResized", fn);
    napi_create_function(env, "windowClosed", NAPI_AUTO_LENGTH, DayWindowClosed, nullptr, &fn);
    napi_set_named_property(env, exports, "windowClosed", fn);
    napi_create_function(env, "windowFocused", NAPI_AUTO_LENGTH, DayWindowFocused, nullptr, &fn);
    napi_set_named_property(env, exports, "windowFocused", fn);
    napi_create_function(env, "onFileResult", NAPI_AUTO_LENGTH, OnFileResult, nullptr, &fn);
    napi_set_named_property(env, exports, "onFileResult", fn);
    napi_create_function(env, "deepLink", NAPI_AUTO_LENGTH, DayDeepLink, nullptr, &fn);
    napi_set_named_property(env, exports, "deepLink", fn);
    napi_create_function(env, "registerResourceManager", NAPI_AUTO_LENGTH, RegisterResourceManager,
                         nullptr, &fn);
    napi_set_named_property(env, exports, "registerResourceManager", fn);
    napi_create_function(env, "registerNav", NAPI_AUTO_LENGTH, RegisterNav, nullptr, &fn);
    napi_set_named_property(env, exports, "registerNav", fn);
    napi_create_function(env, "navPopped", NAPI_AUTO_LENGTH, NavPopped, nullptr, &fn);
    napi_set_named_property(env, exports, "navPopped", fn);
    napi_create_function(env, "navBackRequested", NAPI_AUTO_LENGTH, NavBackRequested, nullptr, &fn);
    napi_set_named_property(env, exports, "navBackRequested", fn);
    napi_create_function(env, "navMenuAction", NAPI_AUTO_LENGTH, NavMenuAction, nullptr, &fn);
    napi_set_named_property(env, exports, "navMenuAction", fn);
    napi_create_function(env, "navPageArea", NAPI_AUTO_LENGTH, NavPageArea, nullptr, &fn);
    napi_set_named_property(env, exports, "navPageArea", fn);
    napi_create_function(env, "registerPiece", NAPI_AUTO_LENGTH, RegisterPiece, nullptr, &fn);
    napi_set_named_property(env, exports, "registerPiece", fn);
    napi_create_function(env, "pieceEvent", NAPI_AUTO_LENGTH, PieceEvent, nullptr, &fn);
    napi_set_named_property(env, exports, "pieceEvent", fn);
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
