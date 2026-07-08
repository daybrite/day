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
#include <mutex>
#include <string>

#include <sys/mman.h>
#include <unistd.h>

#include <arkui/native_interface.h>
#include <arkui/native_node.h>
#include <arkui/native_node_napi.h>
#include <arkui/native_type.h>
#include <napi/native_api.h>
#include <rawfile/raw_file.h>
#include <rawfile/raw_file_manager.h>
#include <uv.h>

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

// The ArkTS-registered file-picker callback (docs/files.md): `(req, mode, name, src, filters)`.
// Held as a napi_ref because HarmonyOS file pickers live in the ArkTS @kit.CoreFileKit layer,
// not the native NodeAPI. Called on the JS thread (day's loop runs there), so no threadsafe fn.
static napi_ref g_file_picker = nullptr;

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
        default:
            break;
    }
}

// day node-kind → ArkUI_NodeType.
static ArkUI_NodeType kind_map(int32_t k) {
    switch (k) {
        case 0: return ARKUI_NODE_STACK;   // container (day owns absolute layout)
        case 1: return ARKUI_NODE_TEXT;    // label
        case 2: return ARKUI_NODE_BUTTON;  // button
        case 3: return ARKUI_NODE_TEXT_INPUT;
        case 4: return ARKUI_NODE_TOGGLE;
        case 5: return ARKUI_NODE_SLIDER;
        case 6: return ARKUI_NODE_SCROLL;
        case 7: return ARKUI_NODE_COLUMN;
        case 8: return ARKUI_NODE_LOADING_PROGRESS;
        case 9: return ARKUI_NODE_IMAGE;  // image (by name, addressed via resource://RAWFILE)
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
void day_ark_set_font_size(void* n, double vp) { set_f32(n, NODE_FONT_SIZE, (float)vp); }
void day_ark_set_font_color(void* n, uint32_t argb) { set_u32(n, NODE_FONT_COLOR, argb); }
void day_ark_set_corner_radius(void* n, double vp) { set_f32(n, NODE_BORDER_RADIUS, (float)vp); }

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
        default: return;
    }
    g_api->registerNodeEvent((ArkUI_NodeHandle)n, t, 0, (void*)(uintptr_t)id);
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
    napi_create_function(env, "onFileResult", NAPI_AUTO_LENGTH, OnFileResult, nullptr, &fn);
    napi_set_named_property(env, exports, "onFileResult", fn);
    napi_create_function(env, "registerResourceManager", NAPI_AUTO_LENGTH, RegisterResourceManager,
                         nullptr, &fn);
    napi_set_named_property(env, exports, "registerResourceManager", fn);
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
