// Woomy transport only. Consume the actual upstream M10 C ABI.
#include <manmenmi/abi/manmenmi_abi.h>
#include <memory>
#include <cstring>
#include <array>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <vector>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif
namespace {
using Bytes=std::vector<std::uint8_t>;
constexpr std::uint32_t protocol=2;
constexpr std::size_t max_frame=65560;
void put(Bytes& b,std::uint32_t v) { for(unsigned n=0;n<4;++n)b.push_back(static_cast<std::uint8_t>(v>>(8*n))); }
std::uint32_t word(const Bytes& b,std::size_t n) {
    if(n>b.size() || b.size()-n<4)throw std::runtime_error("short frame");
    std::uint32_t v=0; for(unsigned i=0;i<4;++i)v|=static_cast<std::uint32_t>(b[n+i])<<(8*i);return v;
}
void send(const Bytes& b) {
    if(b.size()>max_frame)throw std::runtime_error("frame limit");
    Bytes h;put(h,static_cast<std::uint32_t>(b.size()));
    for(auto v:h)std::cout.put(static_cast<char>(v));
    for(auto v:b)std::cout.put(static_cast<char>(v));
    std::cout.flush(); if(!std::cout)throw std::runtime_error("write failed");
}
Bytes receive() {
    Bytes h(4);for(auto& v:h){int c=std::cin.get();if(c==EOF)throw std::runtime_error("input closed");v=static_cast<std::uint8_t>(c);}
    auto n=word(h,0);if(n<4 || n>max_frame)throw std::runtime_error("frame limit");
    Bytes b(n);for(auto& v:b){int c=std::cin.get();if(c==EOF)throw std::runtime_error("short frame");v=static_cast<std::uint8_t>(c);}return b;
}
Bytes message(std::initializer_list<std::uint32_t> words) { Bytes b;for(auto v:words)put(b,v);return b; }
// All callback failures, including C++ transport exceptions, stop at this boundary.
mnm_status validate(void*,mnm_guest_addr a,uint32_t n,uint32_t permissions) noexcept {
    try {
        if(n>65536 || uint64_t(a)+n>0x100000000ull || !permissions || (permissions & ~7u))return MNM_ERR_INVALID_ARGUMENT;
        send(message({10,a,n,permissions}));auto b=receive();
        return b.size()==8 && word(b,0)==13 && word(b,4)==0 ? MNM_OK:MNM_ERR_INVALID_STATE;
    } catch(...) { return MNM_ERR_RUNTIME; }
}
mnm_status read(void*,mnm_guest_addr a,void* dst,uint32_t n) noexcept {
    try {
        if(n>65536 || uint64_t(a)+n>0x100000000ull || (n && !dst))return MNM_ERR_INVALID_ARGUMENT;
        send(message({11,a,n}));auto b=receive();
        if(b.size()!=8+size_t(n) || word(b,0)!=13 || word(b,4)!=0)return MNM_ERR_INVALID_STATE;
        if(n)std::memcpy(dst,b.data()+8,n);return MNM_OK;
    } catch(...) { return MNM_ERR_RUNTIME; }
}
mnm_status write(void*,mnm_guest_addr a,const void* src,uint32_t n) noexcept {
    try {
        if(n>65536 || uint64_t(a)+n>0x100000000ull || (n && !src))return MNM_ERR_INVALID_ARGUMENT;
        auto b=message({12,a,n});auto bytes=static_cast<const uint8_t*>(src);
        if(n)b.insert(b.end(),bytes,bytes+n);send(b);b=receive();
        return b.size()==8 && word(b,0)==13 && word(b,4)==0 ? MNM_OK:MNM_ERR_INVALID_STATE;
    } catch(...) { return MNM_ERR_RUNTIME; }
}
constexpr std::array<const char*,6> names={"OSGetCurrentThread","OSIsDebuggerInitialized","memcpy","OSBlockMove","OSBlockSet","OSReport"};
int run() {
    mnm_version_info version{sizeof(mnm_version_info),0,0,0,0};
    if(mnm_query_version(&version)!=MNM_OK || version.runtime_abi!=1 || version.service_abi!=1 || version.thread_abi!=1 || version.memory_abi!=1)return 2;
    mnm_runtime* raw=nullptr;if(mnm_runtime_create(1,&raw)!=MNM_OK)return 2;
    std::unique_ptr<mnm_runtime,decltype(&mnm_runtime_destroy)> runtime(raw,mnm_runtime_destroy);
    send(message({1,protocol,version.runtime_abi,version.thread_abi,version.service_abi,version.memory_abi}));
    auto init=receive();if(word(init,0)!=2 || init.size()<16)return 3;
    auto debugger=word(init,4),count=word(init,8),current=word(init,12);
    if(debugger>2 || count>32 || init.size()!=16+count*8 || (current!=0xffffffffU && current>=count))return 3;
    mnm_process* process=nullptr;
    if(mnm_runtime_create_process(raw,"Woomy scoped runtime context",&process)!=MNM_OK || mnm_process_set_debugger_state(process,static_cast<mnm_debugger_state>(debugger))!=MNM_OK)return 4;
    for(std::uint32_t i=0;i<count;++i) {
        auto provenance=word(init,20+i*8);if(provenance>2)return 3;
        mnm_thread* thread=nullptr;
        if(mnm_process_create_thread(process,word(init,16+i*8),"Woomy supplied identity",static_cast<mnm_provenance>(provenance),&thread)!=MNM_OK)return 4;
        uint32_t id=0;if(mnm_thread_id(thread,&id)!=MNM_OK)return 4;
        if(i==current && mnm_process_set_current_thread(process,id)!=MNM_OK)return 4;
    }
    auto ready=message({2,0});
    for(const auto* name:names) {
        mnm_capability cap{sizeof(mnm_capability),MNM_SVC_NOT_IMPLEMENTED,MNM_VAL_NOT_IMPLEMENTED,0};
        uint32_t count_services=0;if(mnm_service_count(raw,&count_services)!=MNM_OK || count_services>256)return 4;
        bool found=false;
        for(uint32_t index=0;index<count_services;++index) {
            char library[256]{},symbol[256]{};
            if(mnm_service_at(raw,index,library,256,symbol,256,&cap)!=MNM_OK)return 4;
            if(std::strcmp(library,"coreinit")==0 && std::strcmp(symbol,name)==0) {found=true;break;}
        }
        if(!found)return 4;
        put(ready,static_cast<uint32_t>(cap.status));put(ready,static_cast<uint32_t>(cap.validation));
    }
    send(ready);mnm_memory_callbacks memory{sizeof(mnm_memory_callbacks),MNM_MEMORY_ABI_VERSION,nullptr,validate,read,write};
    for(std::uint32_t calls=0;calls<65536;++calls) {
        auto request=receive();if(word(request,0)==9)return 0;
        if(request.size()!=44 || word(request,0)!=3)return 3;
        auto serial=word(request,4),handler=word(request,8);if(handler<1 || handler>names.size())return 3;
        std::array<std::uint32_t,8> args{};for(std::size_t i=0;i<8;++i)args[i]=word(request,12+i*4);
        mnm_service_call call{sizeof(mnm_service_call),args.data(),static_cast<uint32_t>(args.size()),&memory};
        mnm_service_result result{sizeof(mnm_service_result),0,0};
        auto status=mnm_invoke(raw,"coreinit",names[handler-1],&call,&result);
        send(message({3,serial,static_cast<uint32_t>(status),status==MNM_OK?result.has_return:0u,status==MNM_OK?result.r3:0u}));
    }
    return 5;
}
}
int main() {
#ifdef _WIN32
    if(_setmode(_fileno(stdin),_O_BINARY)==-1 || _setmode(_fileno(stdout),_O_BINARY)==-1)return 6;
#endif
    try { return run(); } catch(const std::exception& e) { std::cerr<<e.what();return 7; }
}
