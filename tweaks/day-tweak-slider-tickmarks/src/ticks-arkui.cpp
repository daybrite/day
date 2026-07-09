// The ArkUI half of day-tweak-slider-tickmarks — the bring-your-own-NDK tweak recipe
// (docs/tweaks.md): the raw `ArkUI_NodeHandle` from `day_arkui::with_native_raw` is driven with
// the ArkUI native node API, resolved here exactly the way day-arkui-sys resolves it. A stepped
// ArkUI slider snaps by nature; NODE_SLIDER_STEP is a percentage of the range.

#include <arkui/native_interface.h>
#include <arkui/native_node.h>
#include <arkui/native_type.h>

extern "C" void day_tweak_slider_ticks_arkui(void* node, float step_percent, int show) {
    ArkUI_NativeNodeAPI_1* api = nullptr;
    OH_ArkUI_GetModuleInterface(ARKUI_NATIVE_NODE, ArkUI_NativeNodeAPI_1, api);
    if (!api || !node) return;
    ArkUI_NumberValue v[1];
    ArkUI_AttributeItem it{ v, 1, nullptr, nullptr };
    v[0].f32 = step_percent;
    api->setAttribute((ArkUI_NodeHandle)node, NODE_SLIDER_STEP, &it);
    v[0].i32 = show ? 1 : 0;
    api->setAttribute((ArkUI_NodeHandle)node, NODE_SLIDER_SHOW_STEPS, &it);
}
