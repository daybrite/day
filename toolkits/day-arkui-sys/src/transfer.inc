// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#include <database/udmf/udmf.h>
#include <database/udmf/uds.h>
#include <memory>
namespace day_transfer {
using Bytes=std::vector<uint8_t>;
using Prepare=unsigned char*(*)(void*,double,double,size_t*);
using Free=void(*)(unsigned char*,size_t);
using Accept=bool(*)(void*,double,double,const char*,bool);
using Receive=bool(*)(void*,double,double,const unsigned char*,size_t,bool);
struct State {Prepare prepare=nullptr;Free release=nullptr;Accept accept=nullptr;Receive receive=nullptr;};
static std::map<ArkUI_NodeHandle,State> states;
static ArkUI_NodeHandle active=nullptr;
constexpr size_t limit=64*1024*1024;
constexpr const char* bundle="application/vnd.day.transfer";
static const char* native(std::string const& mime) {
    if(mime=="image/png")return "general.png";
    if(mime=="image/jpeg")return "general.jpeg";
    if(mime=="text/uri-list")return "general.file-uri";
    return mime.c_str();
}
static std::string mime(std::string const& type) {
    if(type=="general.png")return "image/png";
    if(type=="general.jpeg")return "image/jpeg";
    if(type=="general.file-uri")return "text/uri-list";
    return type;
}
static void u32(Bytes& b,uint32_t n) {for(int i=0;i<4;i++)b.push_back(uint8_t(n>>(8*i)));}
static uint32_t read32(Bytes const& b,size_t& at) {
    if(at+4>b.size())return 0; uint32_t n=0;for(int i=0;i<4;i++)n|=uint32_t(b[at++])<<(8*i);return n;
}
static void add(OH_UdmfData* data,std::string const& type,Bytes const& bytes) {
    auto record=OH_UdmfRecord_Create(); if(!record)return;
    OH_UdmfRecord_AddGeneralEntry(record,native(type),const_cast<uint8_t*>(bytes.data()),bytes.size());
    OH_UdmfData_AddRecord(data,record); OH_UdmfRecord_Destroy(record);
}
static void forget(void* node) {states.erase((ArkUI_NodeHandle)node);if(active==node)active=nullptr;}
static bool event(ArkUI_NodeEvent* ev) {
    auto node=OH_ArkUI_NodeEvent_GetNodeHandle(ev);auto it=states.find(node);
    if(it==states.end())return false;
    auto type=OH_ArkUI_NodeEvent_GetEventType(ev);
    if(type!=NODE_ON_DRAG_START&&type!=NODE_ON_DRAG_END&&type!=NODE_ON_DRAG_ENTER&&type!=NODE_ON_DRAG_MOVE&&type!=NODE_ON_DRAG_LEAVE&&type!=NODE_ON_DROP)return false;
    auto drag=OH_ArkUI_NodeEvent_GetDragEvent(ev);if(!drag)return false;
    const auto s=it->second;
    ArkUI_IntOffset origin{}; OH_ArkUI_NodeUtils_GetPositionWithTranslateInWindow(node,&origin);
    double x=(OH_ArkUI_DragEvent_GetTouchPointXToWindow(drag)-origin.x)/g_density;
    double y=(OH_ArkUI_DragEvent_GetTouchPointYToWindow(drag)-origin.y)/g_density;
    if(type==NODE_ON_DRAG_END) {active=nullptr;return true;}
    if(type==NODE_ON_DRAG_START && s.prepare) {
        size_t n=0;auto p=s.prepare(node,x,y,&n); if(!p)return true;
        Bytes packet(p,p+n);s.release(p,n);if(n>limit)return true;
        auto data=OH_UdmfData_Create(); if(!data)return true;
        add(data,bundle,packet);
        size_t at=8;read32(packet,at);auto count=read32(packet,at);
        for(uint32_t i=0;i<count&&i<32;i++) {
            auto m=read32(packet,at),size=read32(packet,at);if(m>255||size>limit||at+m+size>packet.size())break;
            std::string type(packet.begin()+at,packet.begin()+at+m);at+=m;
            Bytes value(packet.begin()+at,packet.begin()+at+size);at+=size;
            if(type=="text/uri-list") {
                std::string list(value.begin(),value.end());size_t pos=0;
                while(pos<list.size()) {
                    auto end=list.find('\n',pos);if(end==std::string::npos)end=list.size();
                    auto uri=list.substr(pos,end-pos);pos=end+1;if(!uri.empty()&&uri.back()=='\r')uri.pop_back();
                    if(uri.empty()||uri[0]=='#')continue;
                    auto file=OH_UdsFileUri_Create();auto rec=OH_UdmfRecord_Create();
                    if(file&&rec) {OH_UdsFileUri_SetFileUri(file,uri.c_str());OH_UdmfRecord_AddFileUri(rec,file);OH_UdmfData_AddRecord(data,rec);}
                    if(rec)OH_UdmfRecord_Destroy(rec);if(file)OH_UdsFileUri_Destroy(file);
                }
            } else add(data,type,value);
        }
        if(OH_ArkUI_DragEvent_SetData(drag,data)==0)active=node;
        OH_UdmfData_Destroy(data);return true;
    }
    if(!s.accept)return false;
    if(type==NODE_ON_DRAG_LEAVE) {OH_ArkUI_AllowNodeAllDropDataTypes(node);return true;}
    if(type!=NODE_ON_DRAG_ENTER&&type!=NODE_ON_DRAG_MOVE&&type!=NODE_ON_DROP)return false;
    int32_t count=0;OH_ArkUI_DragEvent_GetDataTypeCount(drag,&count);if(count<0||count>256)return true;
    std::vector<std::vector<char>> buffers(count,std::vector<char>(256));std::vector<char*> names;
    for(auto& b:buffers)names.push_back(b.data());
    if(OH_ArkUI_DragEvent_GetDataTypes(drag,names.data(),count,256)!=0)return true;
    std::string types;for(auto n:names)types+=mime(n)+"\n";
    bool accepted=s.accept(node,x,y,types.c_str(),active!=nullptr);
    if(accepted) {OH_ArkUI_AllowNodeAllDropDataTypes(node);OH_ArkUI_DragEvent_SetSuggestedDropOperation(drag,ARKUI_DROP_OPERATION_COPY);}
    else OH_ArkUI_DisallowNodeAnyDropDataTypes(node);
    if(type!=NODE_ON_DROP)return true;
    OH_ArkUI_DragEvent_SetDragResult(drag,ARKUI_DRAG_RESULT_FAILED);
    if(!accepted)return true;
    auto data=OH_UdmfData_Create();if(!data)return true;
    if(OH_ArkUI_DragEvent_GetUdmfData(drag,data)==0) {
        unsigned int count=0;auto records=OH_UdmfData_GetRecords(data,&count);Bytes packet;
        std::vector<std::pair<std::string,Bytes>> reps;size_t total=0;
        for(unsigned int i=0;records&&i<count&&count<=256;i++) {
            unsigned char* p=nullptr;unsigned int n=0;
            if(OH_UdmfRecord_GetGeneralEntry(records[i],bundle,&p,&n)==0&&p&&n<=limit) {packet.assign(p,p+n);break;}
            unsigned int num=0;auto types=OH_UdmfRecord_GetTypes(records[i],&num);
            for(unsigned int j=0;types&&j<num&&num<=32;j++) {
                auto m=mime(types[j]);
                if(m=="text/uri-list") {
                    auto f=OH_UdsFileUri_Create();
                    if(f&&OH_UdmfRecord_GetFileUri(records[i],f)==0) {auto uri=OH_UdsFileUri_GetFileUri(f);if(uri) {std::string v=std::string(uri)+"\r\n";reps.push_back({m,Bytes(v.begin(),v.end())});}}
                    if(f)OH_UdsFileUri_Destroy(f);
                } else if(OH_UdmfRecord_GetGeneralEntry(records[i],types[j],&p,&n)==0&&p&&n<=limit) {total+=n;if(total>limit)break;reps.push_back({m,Bytes(p,p+n)});}
            }
        }
        if(packet.empty()&&total<=limit&&!reps.empty()&&reps.size()<=256) {
            packet={'D','A','Y','D','N','D',0,1};u32(packet,reps.size());
            for(auto const& r:reps) {u32(packet,1);u32(packet,r.first.size());u32(packet,r.second.size());packet.insert(packet.end(),r.first.begin(),r.first.end());packet.insert(packet.end(),r.second.begin(),r.second.end());}
        }
        if(!packet.empty()&&packet.size()<=limit&&s.receive(node,x,y,packet.data(),packet.size(),active!=nullptr))OH_ArkUI_DragEvent_SetDragResult(drag,ARKUI_DRAG_RESULT_SUCCESSFUL);
    }
    OH_UdmfData_Destroy(data);return true;
}
}
extern "C" void day_arkui_drag_source(void* h,day_transfer::Prepare p,day_transfer::Free f) {
    auto node=(ArkUI_NodeHandle)h;auto& s=day_transfer::states[node];s.prepare=p;s.release=f;
    OH_ArkUI_SetNodeDraggable(node,true);g_api->registerNodeEvent(node,NODE_ON_DRAG_START,0,nullptr);g_api->registerNodeEvent(node,NODE_ON_DRAG_END,0,nullptr);
}
extern "C" void day_arkui_drop_target(void* h,day_transfer::Accept a,day_transfer::Receive r) {
    auto node=(ArkUI_NodeHandle)h;auto& s=day_transfer::states[node];s.accept=a;s.receive=r;
    OH_ArkUI_AllowNodeAllDropDataTypes(node);
    for(auto type:{NODE_ON_DRAG_ENTER,NODE_ON_DRAG_MOVE,NODE_ON_DRAG_LEAVE,NODE_ON_DROP})g_api->registerNodeEvent(node,type,0,nullptr);
}
