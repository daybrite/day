// The media piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. day-winui hosts the UWP
// system XAML (winrt::Windows::UI::Xaml, from the base Windows SDK — no WinAppSDK), so the matching
// player is Windows.UI.Xaml.Controls.MediaPlayerElement backed by a Windows.Media.Playback
// MediaPlayer. The element is boxed into a day handle via the `day_winui_box`/`day_winui_unbox` seam
// day-winui-sys exports (zero edits to day's toolkit crates), exactly like the picker/webview shims.
//
// Written blind (no Windows host here); Windows-only, compiled by build.rs and linked alongside
// day-winui-sys. MediaPlayerElement is core system XAML so construction can't fail like EdgeHTML,
// but creation still degrades to a URL TextBlock on any unexpected throw so the app keeps running.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Media.Core.h>     // MediaSource
#include <winrt/Windows.Media.Playback.h> // MediaPlayer
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>

#include <windows.h>

#include <cstdint>
#include <string>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUX = winrt::Windows::UI::Xaml;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WMC = winrt::Windows::Media::Core;
namespace WMP = winrt::Windows::Media::Playback;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

static winrt::hstring hs(const char *s) {
    if (!s || !*s)
        return winrt::hstring{};
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1)
        return winrt::hstring{};
    std::wstring w(static_cast<size_t>(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w.data(), len);
    return winrt::hstring{w};
}

// A MediaSource for a url string: an http(s)/file URL is used directly; a bare local path (no
// scheme) becomes a file:/// URI (backslashes → forward). Returns null on empty/invalid input.
static WMC::MediaSource source_from(const char *url) {
    std::string s = url ? url : "";
    if (s.empty())
        return nullptr;
    if (s.find("://") == std::string::npos) {
        for (auto &ch : s)
            if (ch == '\\')
                ch = '/';
        s = "file:///" + s;
    }
    try {
        return WMC::MediaSource::CreateFromUri(WF::Uri{hs(s.c_str())});
    } catch (...) {
        return nullptr;
    }
}

static WMP::MediaPlayer player_of(void *handle) {
    WUX::UIElement e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    if (auto mpe = e.try_as<WUXC::MediaPlayerElement>())
        return mpe.MediaPlayer();
    return nullptr;
}

extern "C" {

void *day_media_winui_new(const char *url, int autoplay, int looping, int muted, int controls) {
    try {
        WMP::MediaPlayer player;
        player.AutoPlay(autoplay != 0);
        player.IsMuted(muted != 0);
        player.IsLoopingEnabled(looping != 0);
        if (auto src = source_from(url))
            player.Source(src);
        WUXC::MediaPlayerElement mpe;
        mpe.AreTransportControlsEnabled(controls != 0);
        mpe.SetMediaPlayer(player);
        return day_winui_box(winrt::get_abi(mpe));
    } catch (...) {
        // Any unexpected failure — degrade to a label so the app still runs and screenshots.
        WUXC::TextBlock tb;
        tb.Text(hs(url ? url : ""));
        return day_winui_box(winrt::get_abi(tb));
    }
}

void day_media_winui_load(void *handle, const char *url) {
    try {
        if (auto p = player_of(handle)) {
            if (auto src = source_from(url))
                p.Source(src);
            p.Play();
        }
    } catch (...) {
    }
}
void day_media_winui_play(void *handle) {
    try {
        if (auto p = player_of(handle))
            p.Play();
    } catch (...) {
    }
}
void day_media_winui_pause(void *handle) {
    try {
        if (auto p = player_of(handle))
            p.Pause();
    } catch (...) {
    }
}

} // extern "C"
