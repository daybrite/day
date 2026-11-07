// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Included inside shim.cpp's C ABI block. Every drawing goes through paint_canvas,
// whether its destination is a live control or an offscreen PNG.
extern "C++" {
namespace WC = winrt::Microsoft::Graphics::Canvas;
namespace WCG = WC::Geometry;
namespace WCB = WC::Brushes;
namespace WCT = WC::Text;
namespace WCX = WC::UI::Xaml;
namespace WFN = winrt::Windows::Foundation::Numerics;

// Cargo does not run MSBuild's WinRT registration task. Preload Win2D from the
// app-local DLL, with the pinned SDK cache as the development fallback. C++/WinRT's
// normal activation then finds it by namespace. Do not replace winrt_activation_handler:
// delegating only to RoGetActivationFactory bypasses C++/WinRT's DLL fallback and
// breaks other unpackaged components such as WebView2 after the first canvas draws.
static void canvas_activate() {
    static HMODULE module = [] {
        wchar_t path[32768]{};
        GetModuleFileNameW(nullptr, path, 32768);
        std::wstring local(path);
        local.resize(local.find_last_of(L"\\/") + 1);
        local += L"Microsoft.Graphics.Canvas.dll";
        auto dll = LoadLibraryExW(local.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        if (!dll) dll = LoadLibraryExW(DAY_WIN2D_DLL, nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        return dll;
    }();
    if (!module) winrt::throw_hresult(HRESULT_FROM_WIN32(ERROR_MOD_NOT_FOUND));
}

static WC::CanvasDevice canvas_device() {
    canvas_activate();
    return WC::CanvasDevice::GetSharedDevice();
}

static winrt::com_ptr<IWICImagingFactory> canvas_wic() {
    return winrt::create_instance<IWICImagingFactory>(CLSID_WICImagingFactory);
}

struct CanvasPixels { uint32_t width, height; std::vector<uint8_t> bytes; };
static CanvasPixels canvas_decode(std::vector<uint8_t> const& bytes, uint32_t fit_w = 0, uint32_t fit_h = 0) {
    auto factory = canvas_wic();
    winrt::com_ptr<IWICStream> stream;
    winrt::check_hresult(factory->CreateStream(stream.put()));
    winrt::check_hresult(stream->InitializeFromMemory(const_cast<BYTE*>(bytes.data()), static_cast<DWORD>(bytes.size())));
    winrt::com_ptr<IWICBitmapDecoder> decoder;
    winrt::check_hresult(factory->CreateDecoderFromStream(stream.get(), nullptr, WICDecodeMetadataCacheOnLoad, decoder.put()));
    winrt::com_ptr<IWICBitmapFrameDecode> frame;
    winrt::check_hresult(decoder->GetFrame(0, frame.put()));
    uint32_t w, h;
    winrt::check_hresult(frame->GetSize(&w, &h));
    // The same allocation limit as Day's export API, also for imported image buffers.
    if (!w || !h || w > 16384 || h > 16384 || uint64_t(w) * h > 64000000) winrt::throw_hresult(E_INVALIDARG);
    winrt::com_ptr<IWICBitmapSource> source = frame.as<IWICBitmapSource>();
    if (fit_w && fit_h) {
        double scale = (std::min)({1.0, double(fit_w) / w, double(fit_h) / h});
        uint32_t nw = (std::max)(1u, uint32_t(std::round(w * scale)));
        uint32_t nh = (std::max)(1u, uint32_t(std::round(h * scale)));
        if (nw != w || nh != h) {
            winrt::com_ptr<IWICBitmapScaler> scaler;
            winrt::check_hresult(factory->CreateBitmapScaler(scaler.put()));
            winrt::check_hresult(scaler->Initialize(source.get(), nw, nh, WICBitmapInterpolationModeFant));
            source = scaler.as<IWICBitmapSource>(); w = nw; h = nh;
        }
    }
    winrt::com_ptr<IWICFormatConverter> converter;
    winrt::check_hresult(factory->CreateFormatConverter(converter.put()));
    winrt::check_hresult(converter->Initialize(source.get(), GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone, nullptr, 0, WICBitmapPaletteTypeCustom));
    CanvasPixels pixels{w, h, std::vector<uint8_t>(size_t(w) * h * 4)};
    winrt::check_hresult(converter->CopyPixels(nullptr, w * 4, static_cast<UINT>(pixels.bytes.size()), pixels.bytes.data()));
    return pixels;
}

static std::vector<uint8_t> canvas_png(CanvasPixels const& pixels) {
    auto factory = canvas_wic();
    winrt::com_ptr<IStream> stream;
    winrt::check_hresult(CreateStreamOnHGlobal(nullptr, TRUE, stream.put()));
    winrt::com_ptr<IWICBitmapEncoder> encoder;
    winrt::check_hresult(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, encoder.put()));
    winrt::check_hresult(encoder->Initialize(stream.get(), WICBitmapEncoderNoCache));
    winrt::com_ptr<IWICBitmapFrameEncode> frame;
    winrt::check_hresult(encoder->CreateNewFrame(frame.put(), nullptr));
    winrt::check_hresult(frame->Initialize(nullptr));
    winrt::check_hresult(frame->SetSize(pixels.width, pixels.height));
    WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
    winrt::check_hresult(frame->SetPixelFormat(&format));
    // PNG stores straight alpha. Let WIC perform the premultiplied conversion.
    winrt::com_ptr<IWICBitmap> bitmap;
    winrt::check_hresult(factory->CreateBitmapFromMemory(pixels.width, pixels.height,
        GUID_WICPixelFormat32bppPBGRA, pixels.width * 4, static_cast<UINT>(pixels.bytes.size()),
        const_cast<BYTE*>(pixels.bytes.data()), bitmap.put()));
    winrt::com_ptr<IWICFormatConverter> converter;
    winrt::check_hresult(factory->CreateFormatConverter(converter.put()));
    winrt::check_hresult(converter->Initialize(bitmap.get(), format, WICBitmapDitherTypeNone,
        nullptr, 0, WICBitmapPaletteTypeCustom));
    winrt::check_hresult(frame->WriteSource(converter.get(), nullptr));
    winrt::check_hresult(frame->Commit());
    winrt::check_hresult(encoder->Commit());
    STATSTG stat{};
    winrt::check_hresult(stream->Stat(&stat, STATFLAG_NONAME));
    std::vector<uint8_t> bytes(static_cast<size_t>(stat.cbSize.QuadPart));
    winrt::check_hresult(stream->Seek({}, STREAM_SEEK_SET, nullptr));
    ULONG read = 0;
    winrt::check_hresult(stream->Read(bytes.data(), static_cast<ULONG>(bytes.size()), &read));
    if (read != bytes.size()) winrt::throw_hresult(E_FAIL);
    return bytes;
}

static std::map<uint64_t, WC::CanvasBitmap> canvas_images;
static WC::CanvasBitmap canvas_image(WC::CanvasDevice const& device, uint64_t id) {
    auto encoded = xaml_bitmaps().find(id);
    if (encoded == xaml_bitmaps().end()) return nullptr;
    auto found = canvas_images.find(id);
    if (found != canvas_images.end() && found->second.Device() == device) return found->second;
    auto pixels = canvas_decode(encoded->second);
    auto bitmap = WC::CanvasBitmap::CreateFromBytes(device, pixels.bytes, pixels.width, pixels.height,
        winrt::Windows::Graphics::DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized, 96,
        WC::CanvasAlphaMode::Premultiplied);
    canvas_images.insert_or_assign(id, bitmap);
    return bitmap;
}

static WCT::CanvasTextLayout canvas_text(WC::CanvasDevice const& device, std::string const& text,
                                        float size, int weight, bool italic, std::string const& family) {
    WCT::CanvasTextFormat format;
    format.FontSize(size);
    format.FontFamily(hs(family.empty() ? "Segoe UI" : family.c_str()));
    format.FontWeight(winrt::Windows::UI::Text::FontWeight{static_cast<uint16_t>(weight ? weight : 400)});
    format.FontStyle(italic ? winrt::Windows::UI::Text::FontStyle::Italic : winrt::Windows::UI::Text::FontStyle::Normal);
    format.WordWrapping(WCT::CanvasWordWrapping::NoWrap);
    return WCT::CanvasTextLayout(device, hs(text.c_str()), format, 0, 0);
}

static WCG::CanvasGeometry canvas_path(WC::CanvasDevice const& device, GeoData const& data) {
    WCG::CanvasPathBuilder builder(device);
    builder.SetFilledRegionDetermination(data.rule == 1 ? WCG::CanvasFilledRegionDetermination::Alternate
                                                       : WCG::CanvasFilledRegionDetermination::Winding);
    for (auto const& figure : data.figs) {
        builder.BeginFigure(WFN::float2{figure.x, figure.y});
        for (auto const& seg : figure.segs) {
            if (seg.op == 'L') builder.AddLine(WFN::float2{seg.v[0], seg.v[1]});
            else if (seg.op == 'Q') builder.AddQuadraticBezier({seg.v[0], seg.v[1]}, {seg.v[2], seg.v[3]});
            else builder.AddCubicBezier({seg.v[0], seg.v[1]}, {seg.v[2], seg.v[3]}, {seg.v[4], seg.v[5]});
        }
        builder.EndFigure(figure.closed ? WCG::CanvasFigureLoop::Closed : WCG::CanvasFigureLoop::Open);
    }
    return WCG::CanvasGeometry::CreatePath(builder);
}

static GeoData canvas_polygon(std::string const& text) {
    GeoData data; data.rule = 0; data.figs.emplace_back();
    auto& f = data.figs.back(); f.closed = true;
    const char* pos = text.data(); const char* end = pos + text.size();
    bool first = true;
    while (pos < end) {
        while (pos < end && (*pos == ' ' || *pos == ',')) ++pos;
        float x = 0, y = 0;
        auto a = std::from_chars(pos, end, x); if (a.ec != std::errc()) break; pos = a.ptr;
        while (pos < end && (*pos == ' ' || *pos == ',')) ++pos;
        auto b = std::from_chars(pos, end, y); if (b.ec != std::errc()) break; pos = b.ptr;
        if (first) { f.x = x; f.y = y; first = false; }
        else f.segs.push_back({'L', {x, y}});
    }
    return data;
}

struct CanvasDisplayList {
    std::vector<double> nums;
    std::vector<std::string> texts;
    void assign(const double* values, int count, const char* joined) {
        nums.assign(values, values + count); texts.clear();
        std::string all = joined ? joined : "";
        size_t start = 0;
        do {
            size_t next = all.find('\x1f', start);
            texts.push_back(all.substr(start, next == std::string::npos ? next : next - start));
            if (next == std::string::npos) break;
            start = next + 1;
        } while (true);
    }
};

static void paint_canvas(WC::CanvasDrawingSession const& ds, CanvasDisplayList const& ops, float scale = 1) {
    auto device = ds.Device();
    WFN::float3x2 cur = WFN::make_float3x2_scale(scale);
    struct Saved { WFN::float3x2 transform; size_t clips; };
    std::vector<Saved> saved;
    std::vector<WC::CanvasActiveLayer> clips;
    // Close layers in reverse order even if a drawing operation fails.
    struct CloseLayers { std::vector<WC::CanvasActiveLayer>& layers; ~CloseLayers() {
        while (!layers.empty()) { try { layers.back().Close(); } catch (...) {} layers.pop_back(); }
    }} close_layers{clips};
    std::vector<WFN::float2> stamps;
    size_t stamp_count = 0, ti = 0;
    auto text = [&]() { return ti < ops.texts.size() ? ops.texts[ti++] : std::string(); };
    std::vector<WCB::CanvasGradientStop> stops;
    float gx=0, gy=0, gz=0, gw=0; int gradient_kind = 0;
    WCG::CanvasStrokeStyle style{nullptr};
    std::vector<float> style_dash; float style_phase = 0;
    int weight = 0; bool italic = false; std::string family;
    // A kind-23 record: the region, in the bitmap's pixels, the next image record draws.
    bool has_source = false; WF::Rect source{};
    for (size_t i = 0; i + 8 < ops.nums.size(); i += 9) {
        auto p = ops.nums.data() + i;
        int k = int(p[0]); float a=float(p[1]), b=float(p[2]), c=float(p[3]), d=float(p[4]);
        float e=float(p[5]), f=float(p[6]), g=float(p[7]); auto color = color_argb(static_cast<unsigned>(p[8]));
        if (k == 20) { stamps.clear(); stamp_count = size_t(a); continue; }
        if (k == 21) { for (int j=0; j<4 && stamps.size()<stamp_count; ++j) stamps.push_back({float(p[1+j*2]),float(p[2+j*2])}); continue; }
        if (k == 8) { saved.push_back({cur, clips.size()}); continue; }
        if (k == 9) {
            if (!saved.empty()) {
                while (clips.size() > saved.back().clips) { clips.back().Close(); clips.pop_back(); }
                cur = saved.back().transform; saved.pop_back();
            }
            continue;
        }
        if (k == 10) { cur = WFN::float3x2{a,b,c,d,e,f} * cur; continue; }
        if (k == 19) { weight=int(a); italic=b!=0; family=text(); continue; }
        if (k == 23) { source=WF::Rect{a,b,c,d}; has_source=true; continue; }
        if (k == 18) {
            style = WCG::CanvasStrokeStyle();
            auto cap = int(a)==1 ? WCG::CanvasCapStyle::Round : int(a)==2 ? WCG::CanvasCapStyle::Square : WCG::CanvasCapStyle::Flat;
            style.StartCap(cap); style.EndCap(cap); style.DashCap(cap);
            style.LineJoin(int(b)==1 ? WCG::CanvasLineJoin::Round : int(b)==2 ? WCG::CanvasLineJoin::Bevel : WCG::CanvasLineJoin::Miter);
            style.MiterLimit(c); style_phase = d;
            auto payload = text(); std::vector<float> dash;
            const char* at=payload.data(); const char* end=at+payload.size();
            while (at<end) { if (*at==' ') { ++at; continue; } float v=0; auto parsed=std::from_chars(at,end,v); if(parsed.ec!=std::errc())break; dash.push_back(v); at=parsed.ptr; }
            style_dash = std::move(dash);
            continue;
        }
        if (k == 14) {
            gx=a; gy=b; gz=c; gw=d; gradient_kind=int(f); stops.clear();
            auto payload=text(); const char* at=payload.data(); const char* end=at+payload.size();
            while(at<end) {
                if (*at==' ') { ++at; continue; }
                float offset=0; unsigned bits=0;
                auto x=std::from_chars(at,end,offset); if(x.ec!=std::errc() || x.ptr==end)break;
                auto y=std::from_chars(x.ptr+1,end,bits,16); if(y.ec!=std::errc())break;
                stops.push_back({offset,color_argb(bits)}); at=y.ptr;
            }
            std::stable_sort(stops.begin(),stops.end(),[](auto const& x,auto const& y){return x.Position<y.Position;});
            continue;
        }
        const size_t payload_start=ti;
        size_t repetitions=stamps.empty()?1:stamps.size();
        for(size_t rep=0;rep<repetitions;++rep) {
            ti=payload_start;
            ds.Transform(stamps.empty()?cur:WFN::make_float3x2_translation(stamps[rep])*cur);
            WCG::CanvasGeometry geometry{nullptr};
            bool stroke = k==1 || k==4 || k==5 || k==6 || k==12 || k==13 || k==16;
            int shape = k;
            if(k==17) shape = int(f)==0?0:int(f)==1?2:int(f)==2?3:int(f)==3?15:11;
            if(shape==0 || shape==1) geometry=WCG::CanvasGeometry::CreateRectangle(device, a,b,c,d);
            else if(shape==2 || shape==13) geometry=WCG::CanvasGeometry::CreateRoundedRectangle(device, a,b,c,d,e,e);
            else if(shape==3 || shape==4) geometry=WCG::CanvasGeometry::CreateEllipse(device,a+c/2,b+d/2,c/2,d/2);
            else if(shape==11 || shape==12) geometry=canvas_path(device,canvas_polygon(text()));
            else if(shape==15 || shape==16) geometry=canvas_path(device,parse_path_data(text(),int(k==17?e:f)));
            else if(shape==5 || shape==6) {
                WCG::CanvasPathBuilder path(device);
                if(shape==6) { path.BeginFigure({a,b}); path.AddLine({c,d}); }
                else {
                    constexpr float radians=3.14159265358979323846f/180;
                    // Center/radii arc API handles full revolutions and either sweep direction.
                    path.BeginFigure({a+c/2+c/2*std::cos(e*radians),b+d/2+d/2*std::sin(e*radians)});
                    path.AddArc({a+c/2,b+d/2},c/2,d/2,e*radians,f*radians);
                }
                path.EndFigure(WCG::CanvasFigureLoop::Open); geometry=WCG::CanvasGeometry::CreatePath(path);
            }
            if(k==17) { if(geometry) clips.push_back(ds.CreateLayer(1.0f,geometry)); continue; }
            if(geometry) {
                WCB::ICanvasBrush brush=WCB::CanvasSolidColorBrush(device,color);
                if(stops.size()>=2) {
                    auto bounds=geometry.ComputeBounds();
                    if(gradient_kind==1) {
                        WCB::CanvasRadialGradientBrush gradient(device,stops);
                        gradient.Center({bounds.X+gx*bounds.Width,bounds.Y+gy*bounds.Height});
                        gradient.RadiusX(gz*bounds.Width); gradient.RadiusY(gz*bounds.Height); brush=gradient;
                    } else {
                        WCB::CanvasLinearGradientBrush gradient(device,stops);
                        gradient.StartPoint({bounds.X+gx*bounds.Width,bounds.Y+gy*bounds.Height});
                        gradient.EndPoint({bounds.X+gz*bounds.Width,bounds.Y+gw*bounds.Height}); brush=gradient;
                    }
                }
                if(stroke && g>0) {
                    auto actual=style;
                    if(!actual) { actual=WCG::CanvasStrokeStyle(); if(k==5 || k==6) { actual.StartCap(WCG::CanvasCapStyle::Round); actual.EndCap(WCG::CanvasCapStyle::Round); } }
                    // Day dashes are absolute lengths; Direct2D's are multiples of stroke width.
                    if(style) { auto dash=style_dash; for(auto& value:dash)value/=g; actual.CustomDashStyle(dash); actual.DashOffset(style_phase/g); }
                    ds.DrawGeometry(geometry,brush,g,actual);
                } else if(!stroke) ds.FillGeometry(geometry,brush);
            } else if(k==7) {
                auto value=text(); if(!(e>0) || !std::isfinite(e))continue;
                auto layout=canvas_text(device,value,e,weight,italic,family);
                auto bounds=layout.LayoutBoundsIncludingTrailingWhitespace(); auto lines=layout.LineMetrics();
                int horizontal=int(f)/4, vertical=int(f)%4;
                float x=a-(horizontal==1?bounds.Width/2:horizontal==2?bounds.Width:0);
                float y=b-(vertical==1?bounds.Height/2:vertical==2?(lines.empty()?0:lines[0].Baseline):vertical==3?bounds.Height:0);
                ds.DrawTextLayout(layout,{x,y},color);
            } else if(k==22 && c>0 && d>0) {
                if(auto bitmap=canvas_image(device,static_cast<uint64_t>(p[5]))) {
                    // The bitmap is created at 96 DPI, so its bounds are its pixels: the space
                    // a region names. The region is clamped to the bitmap with its mapping into
                    // the rect kept (day_spec::clamp_image_source); an empty intersection
                    // draws nothing.
                    auto bounds=bitmap.Bounds(); WF::Rect dest{a,b,c,d}; WF::Rect src=bounds; bool draw=true;
                    if(has_source) {
                        float x0=(std::max)(0.0f,source.X), y0=(std::max)(0.0f,source.Y);
                        float x1=(std::min)(bounds.Width,source.X+source.Width), y1=(std::min)(bounds.Height,source.Y+source.Height);
                        draw=source.Width>0 && source.Height>0 && x1>x0 && y1>y0;
                        if(draw) {
                            float kx=c/source.Width, ky=d/source.Height;
                            dest=WF::Rect{a+(x0-source.X)*kx,b+(y0-source.Y)*ky,(x1-x0)*kx,(y1-y0)*ky};
                            src=WF::Rect{x0,y0,x1-x0,y1-y0};
                        }
                    }
                    if(draw) ds.DrawImage(bitmap,dest,src,std::clamp(f,0.0f,1.0f));
                }
            }
        }
        stamps.clear(); stops.clear(); style=nullptr; weight=0; italic=false; family.clear(); has_source=false;
    }
}

struct CanvasState {
    WCX::CanvasControl control{nullptr}; CanvasDisplayList ops; winrt::event_token draw;
    // The high half of a surrogate pair `CharacterReceived` delivered, waiting for its low half
    // (docs/menus.md "The full route"): a supplementary character arrives as two events.
    wchar_t pending_high=0;
};
static std::map<void*,std::shared_ptr<CanvasState>> canvas_states;
static void canvas_forget(void* h) {
    auto it=canvas_states.find(h); if(it==canvas_states.end())return;
    it->second->control.Draw(it->second->draw);
    it->second->control.RemoveFromVisualTree();
    canvas_states.erase(it);
}
} // extern C++

// The full route (docs/menus.md "The full route"). `KeyDown` names the keys that type nothing
// and the chords; `CharacterReceived` delivers the text, once, after the layout and the input
// method have composed it, so nothing is heard twice: a character that reached `KeyDown` under
// Ctrl or Alt never arrives there (Windows swallows it), and one that arrives there was never
// named here.
static std::string day_xaml_named_key(winrt::Windows::System::VirtualKey k) {
    using VK = winrt::Windows::System::VirtualKey;
    switch (k) {
        case VK::Left:  return "ArrowLeft";
        case VK::Right: return "ArrowRight";
        case VK::Up:    return "ArrowUp";
        case VK::Down:  return "ArrowDown";
        case VK::Enter: return "Enter";
        case VK::Tab:   return "Tab";
        case VK::Escape: return "Escape";
        case VK::Back:  return "Backspace";
        case VK::Delete: return "Delete";
        case VK::Home:  return "Home";
        case VK::End:   return "End";
        case VK::PageUp: return "PageUp";
        case VK::PageDown: return "PageDown";
        case VK::Insert: return "Insert";
        default: break;
    }
    if (k >= VK::F1 && k <= VK::F12) return "F" + std::to_string(static_cast<int>(k) - static_cast<int>(VK::F1) + 1);
    return std::string();
}

// AltGr is Ctrl+Alt to Windows, with the right Alt down: it types (the @ on a German layout),
// so it is not a chord, and `CharacterReceived` names what it typed.
static bool day_xaml_altgr_held() {
    return (GetKeyState(VK_CONTROL)&0x8000) && (GetKeyState(VK_RMENU)&0x8000);
}

// The character a virtual key names under a Ctrl or Alt chord: the letters cased by Shift, and
// every other key by what it types unshifted on the current layout (`MapVirtualKeyW`, so Ctrl+[
// names "[" and Ctrl+\ names "\" on a layout that has them where the US one does, and whatever
// the user's layout puts there otherwise). Empty for a key with no character of its own.
static std::string day_xaml_chord_name(winrt::Windows::System::VirtualKey k, int mods) {
    const UINT code = static_cast<UINT>(k);
    if (code >= 'A' && code <= 'Z') return std::string(1, static_cast<char>((mods & 1) ? code : code + 32));
    const UINT ch = MapVirtualKeyW(code, MAPVK_VK_TO_CHAR) & 0xffff;
    if (ch < 0x20 || ch == 0x7f) return std::string();
    return winrt::to_string(std::wstring(1, static_cast<wchar_t>(ch)));
}

void* day_xaml_canvas_new(unsigned long long id, int (*handles)(unsigned long long),
                          int (*handles_text)(unsigned long long),
                          void (*cb)(unsigned long long,const char*,const char*,int)) {
    canvas_activate();
    auto state=std::make_shared<CanvasState>();
    state->control=WCX::CanvasControl();
    state->control.ClearColor(color_argb(0));
    state->control.IsHitTestVisible(false);
    state->control.UseSharedDevice(true);
    std::weak_ptr<CanvasState> weak=state;
    state->draw=state->control.Draw([weak](auto const&,WCX::CanvasDrawEventArgs const& args){
        if(auto state=weak.lock()) paint_canvas(args.DrawingSession(),state->ops);
    });
    WUXC::Grid surface; surface.Background(WUXM::SolidColorBrush(color_argb(0)));
    surface.Children().Append(state->control);
    WUXC::ContentControl host; host.IsTabStop(true);
    host.HorizontalContentAlignment(WUX::HorizontalAlignment::Stretch);
    host.VerticalContentAlignment(WUX::VerticalAlignment::Stretch); host.Content(surface);
    host.PointerPressed([](auto const& sender,auto const&){sender.template as<WUXC::Control>().Focus(WUX::FocusState::Pointer);});
    host.KeyDown([id,handles,handles_text,cb](auto const&,WUXIn::KeyRoutedEventArgs const& args){
        int mods=(GetKeyState(VK_SHIFT)&0x8000?1:0)|(GetKeyState(VK_CONTROL)&0x8000?2:0)|(GetKeyState(VK_MENU)&0x8000?4:0);
        if(!handles(id))return;
        if(handles_text(id)){
            // Ctrl is both the primary key and the Control key here (docs/menus.md). A Win
            // key chord is the system's, and goes on its way.
            if((GetKeyState(VK_LWIN)&0x8000)||(GetKeyState(VK_RWIN)&0x8000))return;
            if(mods&2)mods|=8;
            const std::string named=day_xaml_named_key(args.Key());
            if(!named.empty()){ cb(id,named.c_str(),nullptr,mods); args.Handled(true); return; }
            // A chord names its key and types nothing; everything else types, and
            // `CharacterReceived` delivers it. AltGr types.
            if((mods&(2|4)) && !day_xaml_altgr_held()){
                const std::string key=day_xaml_chord_name(args.Key(),mods);
                if(!key.empty()) cb(id,key.c_str(),nullptr,mods);
                args.Handled(true);
            }
            return;
        }
        const char* key=day_xaml_key_name(args.Key(),mods); if(!key)return;
        cb(id,key,nullptr,mods); args.Handled(true);
    });
    // The text a key types, as text (the full route), the two halves of a surrogate pair joined
    // into the one character they spell.
    host.CharacterReceived([id,handles_text,cb,state](auto const&,WUXIn::CharacterReceivedRoutedEventArgs const& args){
        if(!handles_text(id))return;
        const wchar_t c=args.Character();
        std::wstring text;
        if(c>=0xd800&&c<=0xdbff){ state->pending_high=c; args.Handled(true); return; }
        if(c>=0xdc00&&c<=0xdfff){
            if(!state->pending_high){ args.Handled(true); return; }
            text.push_back(state->pending_high); text.push_back(c); state->pending_high=0;
        } else {
            state->pending_high=0;
            if(c<0x20||c==0x7f)return;
            text.push_back(c);
        }
        // Shift is the only modifier a typed character carries: an AltGr character typed
        // plainly as far as the piece is concerned, and a Ctrl or Alt chord never gets here.
        const int mods=(GetKeyState(VK_SHIFT)&0x8000?1:0);
        const std::string utf8=winrt::to_string(text);
        // One character names itself; a pair is one character too.
        cb(id,utf8.c_str(),utf8.c_str(),mods); args.Handled(true);
    });
    // A chord the piece claims wins over an application accelerator (docs/menus.md "The full
    // route"): XAML asks the focused element first, and a canvas that took the whole keyboard
    // says it handled the accelerator, so a terminal's Ctrl+C is the terminal's.
    host.ProcessKeyboardAccelerators([id,handles_text](auto const&,WUXIn::ProcessKeyboardAcceleratorEventArgs const& args){
        if(!handles_text(id))return;
        if((GetKeyState(VK_LWIN)&0x8000)||(GetKeyState(VK_RWIN)&0x8000))return;
        args.Handled(true);
    });
    auto h=boxh(host); canvas_states.emplace(h,state); return h;
}

void day_xaml_canvas_set_ops(void* h,const double* nums,int n,const char* texts) try {
    auto it=canvas_states.find(h); if(it==canvas_states.end())return;
    it->second->ops.assign(nums,n,texts); it->second->control.Invalidate();
} catch(winrt::hresult_error const& e) { std::fprintf(stderr,"day-toolkit-winui: canvas update failed: %ls\n",e.message().c_str()); }

int day_xaml_canvas_bitmap(unsigned long long id,const double* nums,int n,const char* texts,
                           int width,int height,double scale) try {
    auto device=canvas_device();
    WC::CanvasRenderTarget target(device,float(width),float(height),96);
    CanvasDisplayList ops; ops.assign(nums,n,texts);
    auto ds=target.CreateDrawingSession(); ds.Clear(color_argb(0)); paint_canvas(ds,ops,float(scale)); ds.Close();
    auto pixels=target.GetPixelBytes();
    auto size=target.SizeInPixels();
    if(size.Width!=unsigned(width)||size.Height!=unsigned(height))return 2;
    xaml_bitmaps()[id]=canvas_png({unsigned(width),unsigned(height),{pixels.begin(),pixels.end()}});
    canvas_images.erase(id);
    return 0;
} catch(winrt::hresult_error const& e) { std::fprintf(stderr,"day-toolkit-winui: bitmap render failed: %ls\n",e.message().c_str()); return 1; }
  catch(...) { return 1; }

unsigned char* day_xaml_encode_png(unsigned long long id,int fit_w,int fit_h,int* len) try {
    *len=0; auto it=xaml_bitmaps().find(id); if(it==xaml_bitmaps().end())return nullptr;
    auto bytes=canvas_png(canvas_decode(it->second,fit_w,fit_h));
    auto result=static_cast<unsigned char*>(malloc(bytes.size())); if(!result)return nullptr;
    memcpy(result,bytes.data(),bytes.size()); *len=static_cast<int>(bytes.size()); return result;
} catch(...) { return nullptr; }
void day_xaml_bytes_free(unsigned char* p) { free(p); }

int day_xaml_measure_text(const char* text,double size,int weight,int italic,const char* family,double* out) try {
    if(!(size>0)||!std::isfinite(size))return 1;
    auto layout=canvas_text(canvas_device(),text?text:"",float(size),weight,italic!=0,family?family:"");
    auto box=layout.LayoutBoundsIncludingTrailingWhitespace(); auto ink=layout.DrawBounds(); auto lines=layout.LineMetrics();
    out[0]=box.Width; out[1]=box.Height; out[2]=lines.empty()?0:lines[0].Baseline;
    // Measure H with the same format for cap ink height, including a bundled face.
    auto cap=canvas_text(canvas_device(),"H",float(size),weight,italic!=0,family?family:"").DrawBounds();
    out[3]=cap.Height; out[4]=ink.X; out[5]=ink.Y; out[6]=ink.Width; out[7]=ink.Height; return 0;
} catch(...) { return 1; }
