// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Included outside extern "C": C++/WinRT coroutines retain their deferrals and streams.
namespace day_transfer {
namespace DT = winrt::Windows::ApplicationModel::DataTransfer;
namespace SS = winrt::Windows::Storage::Streams;
using Bytes=std::vector<uint8_t>;
constexpr size_t limit=64*1024*1024;
constexpr auto bundle=L"application/vnd.day.transfer";
using Prepare=unsigned char*(*)(void*,double,double,size_t*);
using Free=void(*)(unsigned char*,size_t);
using Accept=bool(*)(void*,double,double,const char*,bool);
using Receive=bool(*)(void*,double,double,const unsigned char*,size_t,bool);
struct State {
    void* handle;
    bool alive=true;
    Prepare prepare=nullptr; Free release=nullptr; Accept accept=nullptr; Receive receive=nullptr;
};
static std::map<void*,std::shared_ptr<State>> states;
static winrt::hstring local_session;
static std::shared_ptr<State> state(void* h) {
    auto &s=states[h]; if(!s) { s=std::make_shared<State>(); s->handle=h; } return s;
}
static bool local(DT::DataPackageView const& data) {
    try { return !local_session.empty() && data.Properties().HasKey(L"dev.day.session") && winrt::unbox_value<winrt::hstring>(data.Properties().Lookup(L"dev.day.session"))==local_session; } catch(...) { return false; }
}
static uint32_t read_number(Bytes const& b,size_t& at) {
    if(at+4>b.size()) throw std::runtime_error("truncated transfer");
    uint32_t v=0; for(int i=0;i<4;i++) v|=uint32_t(b[at++])<<(8*i); return v;
}
static void number(Bytes &b,uint32_t n) { for(int i=0;i<4;i++) b.push_back(uint8_t(n>>(8*i))); }
using Rep=std::pair<std::string,Bytes>;
static Bytes packet(std::vector<Rep> const& reps) {
    Bytes out={'D','A','Y','D','N','D',0,1}; number(out,1); number(out,uint32_t(reps.size()));
    for(auto const& [mime,bytes]:reps) { number(out,uint32_t(mime.size())); number(out,uint32_t(bytes.size())); out.insert(out.end(),mime.begin(),mime.end()); out.insert(out.end(),bytes.begin(),bytes.end()); }
    return out;
}
static WF::IAsyncOperation<SS::IRandomAccessStream> stream(Bytes bytes) {
    SS::InMemoryRandomAccessStream out; SS::DataWriter writer(out);
    writer.WriteBytes(bytes); co_await writer.StoreAsync(); writer.DetachStream(); out.Seek(0); co_return out;
}
static WF::IAsyncOperation<SS::IBuffer> read(SS::IRandomAccessStream stream) {
    auto size=stream.Size(); if(size>limit) throw std::runtime_error("transfer too large");
    SS::Buffer b{uint32_t(size)}; co_return co_await stream.ReadAsync(b,uint32_t(size),SS::InputStreamOptions::None);
}
static Bytes bytes(SS::IBuffer b) { Bytes out(b.Length()); SS::DataReader::FromBuffer(b).ReadBytes(out); return out; }
static winrt::fire_and_forget start(std::shared_ptr<State> s,WUX::DragStartingEventArgs args,WUX::DragOperationDeferral deferral,Bytes data) {
    try {
        auto package=args.Data(); package.RequestedOperation(DT::DataPackageOperation::Copy);
        GUID session; winrt::check_hresult(CoCreateGuid(&session));
        local_session=winrt::to_hstring(winrt::guid(session));
        package.Properties().Insert(L"dev.day.session",winrt::box_value(local_session));
        package.SetData(bundle,co_await stream(data));
        size_t at=8; read_number(data,at); auto count=read_number(data,at);
        for(uint32_t i=0;i<count && i<32;i++) {
            auto m=read_number(data,at), n=read_number(data,at);
            if(m>255 || n>limit || at+m+n>data.size()) throw std::runtime_error("invalid transfer");
            std::string mime(data.begin()+at,data.begin()+at+m); at+=m;
            Bytes value(data.begin()+at,data.begin()+at+n); at+=n;
            if(mime=="text/uri-list") {
                auto files=winrt::single_threaded_vector<winrt::Windows::Storage::IStorageItem>();
                std::string list(value.begin(),value.end()); size_t pos=0;
                while(pos<list.size()) {
                    auto end=list.find('\n',pos); if(end==std::string::npos)end=list.size();
                    auto line=list.substr(pos,end-pos); if(!line.empty() && line.back()=='\r')line.pop_back(); pos=end+1;
                    if(line.empty() || line[0]=='#')continue;
                    WF::Uri uri(winrt::to_hstring(line));
                    if(uri.SchemeName()!=L"file") continue;
                    auto path=WF::Uri::UnescapeComponent(uri.Path());
                    if(path.size()>2 && path[0]=='/' && path[2]==':')path=winrt::hstring(std::wstring_view(path).substr(1));
                    files.Append(co_await winrt::Windows::Storage::StorageFile::GetFileFromPathAsync(path));
                }
                if(files.Size()) package.SetStorageItems(files);
            } else if(mime=="text/plain") package.SetText(winrt::to_hstring(std::string(value.begin(),value.end())));
            auto native=co_await stream(std::move(value));
            package.SetData(winrt::to_hstring(mime),native);
            if(mime=="image/png") package.SetBitmap(SS::RandomAccessStreamReference::CreateFromStream(native));
        }
        if(!s->alive) args.Cancel(true);
    } catch(...) { args.Cancel(true); local_session={}; }
    deferral.Complete();
}
static std::string types(DT::DataPackageView const& data) {
    std::string result;
    for(auto const& type:data.AvailableFormats()) result+=winrt::to_string(type)+"\n";
    if(data.Contains(DT::StandardDataFormats::StorageItems())) result+="text/uri-list\n";
    if(data.Contains(DT::StandardDataFormats::Bitmap())) result+="image/png\n";
    if(data.Contains(DT::StandardDataFormats::Text())) result+="text/plain\n";
    return result;
}
static bool proposal(std::shared_ptr<State> const& s,WUX::DragEventArgs const& e,WUX::UIElement const& el) {
    auto p=e.GetPosition(el); auto t=types(e.DataView());
    return s->alive && s->accept && (e.AllowedOperations()&DT::DataPackageOperation::Copy)!=DT::DataPackageOperation::None && s->accept(s->handle,p.X,p.Y,t.c_str(),local(e.DataView()));
}
static winrt::fire_and_forget drop(std::shared_ptr<State> s,WUX::DragEventArgs e,WUX::DragOperationDeferral deferral,WF::Point p) {
    bool accepted=false;
    try {
        auto data=e.DataView(); const bool is_local=local(data); Bytes payload;
        if(data.Contains(bundle)) payload=bytes(co_await read((co_await data.GetDataAsync(bundle)).as<SS::IRandomAccessStream>()));
        else {
            std::vector<Rep> reps; size_t total=0;
            if(data.Contains(DT::StandardDataFormats::StorageItems())) {
                std::string uris;
                for(auto const& file:co_await data.GetStorageItemsAsync()) {
                    auto path=winrt::to_string(file.Path()); if(path.empty()) continue;
                    std::replace(path.begin(),path.end(),'\\','/');
                    std::string escaped; constexpr char hex[]="0123456789ABCDEF";
                    for(unsigned char c:path) { if(isalnum(c)||c=='/'||c==':'||c=='-'||c=='_'||c=='.'||c=='~')escaped+=char(c); else {escaped+='%';escaped+=hex[c>>4];escaped+=hex[c&15];} }
                    uris+="file:///"+escaped+"\r\n";
                }
                reps.push_back({"text/uri-list",Bytes(uris.begin(),uris.end())});
            }
            for(auto const& type:data.AvailableFormats()) {
                auto mime=winrt::to_string(type); if(mime.find('/')==std::string::npos || mime.size()>255) continue;
                try {
                    auto v=(co_await data.GetDataAsync(type)).try_as<SS::IRandomAccessStream>();
                    if(v) { auto b=bytes(co_await read(v)); total+=b.size(); if(total>limit||reps.size()>=32)throw std::runtime_error("transfer too large"); reps.push_back({mime,std::move(b)}); }
                } catch(winrt::hresult_error const&) { /* Another representation may be readable. */ }
            }
            if(reps.empty() && data.Contains(DT::StandardDataFormats::Bitmap())) {
                auto ref=co_await data.GetBitmapAsync(); auto image=co_await ref.OpenReadAsync();
                reps.push_back({"image/png",bytes(co_await read(image))});
            }
            payload=packet(reps);
        }
        accepted=s->alive && payload.size()<=limit && s->receive(s->handle,p.X,p.Y,payload.data(),payload.size(),is_local);
    } catch(...) {}
    e.AcceptedOperation(accepted ? DT::DataPackageOperation::Copy : DT::DataPackageOperation::None);
    deferral.Complete();
}
}
extern "C" void day_xaml_drag_source(void* h,day_transfer::Prepare prepare,day_transfer::Free release) {
    guard([&] {
        auto s=day_transfer::state(h); s->prepare=prepare; s->release=release;
        auto el=elem(h); el.CanDrag(true);
        el.DragStarting([s](WUX::UIElement const& el,WUX::DragStartingEventArgs const& args) {
            try {
                if(!s->alive) {args.Cancel(true);return;}
                auto p=args.GetPosition(el); size_t n=0; auto b=s->prepare(s->handle,p.X,p.Y,&n);
                if(!b) {args.Cancel(true);return;}
                day_transfer::Bytes data(b,b+n); s->release(b,n);
                day_transfer::start(s,args,args.GetDeferral(),std::move(data));
            } catch(...) {args.Cancel(true);}
        });
        el.DropCompleted([](auto const&,auto const&) {day_transfer::local_session={};});
    });
}
extern "C" void day_xaml_drop_target(void* h,day_transfer::Accept accept,day_transfer::Receive receive) {
    guard([&] {
        auto s=day_transfer::state(h); s->accept=accept; s->receive=receive;
        auto el=elem(h); el.AllowDrop(true);
        if(auto panel=el.try_as<WUXC::Panel>()) panel.Background(WUXM::SolidColorBrush(WUI::Colors::Transparent()));
        auto over=[s](WF::IInspectable const& sender,WUX::DragEventArgs const& e) {
            try {bool ok=day_transfer::proposal(s,e,sender.as<WUX::UIElement>()); e.AcceptedOperation(ok ? day_transfer::DT::DataPackageOperation::Copy : day_transfer::DT::DataPackageOperation::None); if(ok)e.Handled(true);} catch(...) {e.AcceptedOperation(day_transfer::DT::DataPackageOperation::None);}
        };
        el.DragEnter(over); el.DragOver(over);
        el.Drop([s](WF::IInspectable const& sender,WUX::DragEventArgs const& e) {
            try {auto el=sender.as<WUX::UIElement>(); if(!day_transfer::proposal(s,e,el))return; e.Handled(true); day_transfer::drop(s,e,e.GetDeferral(),e.GetPosition(el));} catch(...) {e.AcceptedOperation(day_transfer::DT::DataPackageOperation::None);}
        });
    });
}
extern "C" void day_xaml_transfer_release(void* h) {
    auto it=day_transfer::states.find(h); if(it!=day_transfer::states.end()) { it->second->alive=false; day_transfer::states.erase(it); }
}
