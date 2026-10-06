// Original Woomy glue over the actual M10 ABI. No service implementations.
#include <manmenmi/abi/manmenmi_abi.h>
#include <cstring>
struct MnmOwner {
    mnm_runtime* runtime=nullptr;
    mnm_process* process=nullptr;
    MnmOwner()=default;
    MnmOwner(const MnmOwner&)=delete;
    MnmOwner& operator=(const MnmOwner&)=delete;
    ~MnmOwner() { mnm_runtime_destroy(runtime); }
    bool create() {
        mnm_version_info v{sizeof(mnm_version_info),0,0,0,0};
        return mnm_query_version(&v)==MNM_OK && v.runtime_abi==1 && v.service_abi==1 && v.thread_abi==1 && v.memory_abi==1
            && mnm_runtime_create(1,&runtime)==MNM_OK && mnm_runtime_create_process(runtime,"Woomy native model",&process)==MNM_OK;
    }
    bool thread(uint32_t address,uint32_t provenance,bool current) {
        mnm_thread* t=nullptr;uint32_t id=0;
        return mnm_process_create_thread(process,address,"Woomy supplied identity",static_cast<mnm_provenance>(provenance),&t)==MNM_OK
            && mnm_thread_id(t,&id)==MNM_OK && (!current || mnm_process_set_current_thread(process,id)==MNM_OK);
    }
    static mnm_status validate(void* context,mnm_guest_addr address,uint32_t length,uint32_t permissions) noexcept {
        try { size_t i,n; if(!context || !length || length>65536 || !permissions || (permissions & ~7u))return MNM_ERR_INVALID_ARGUMENT;
            return location(*static_cast<State*>(context),address,length,permissions,i,n)?MNM_OK:MNM_ERR_INVALID_STATE;
        } catch(...) { return MNM_ERR_RUNTIME; }
    }
    static mnm_status read(void* context,mnm_guest_addr address,void* destination,uint32_t length) noexcept {
        try { if(!destination || validate(context,address,length,1u)!=MNM_OK)return MNM_ERR_INVALID_ARGUMENT;
            auto& s=*static_cast<State*>(context);size_t i,n;if(!location(s,address,length,1u,i,n))return MNM_ERR_INVALID_STATE;
            const auto& bytes=i==SIZE_MAX?s.memory:s.regions[i].bytes;std::memcpy(destination,bytes.data()+n,length);return MNM_OK;
        } catch(...) { return MNM_ERR_RUNTIME; }
    }
    static mnm_status write(void* context,mnm_guest_addr address,const void* source,uint32_t length) noexcept {
        try { if(!source || validate(context,address,length,2u)!=MNM_OK)return MNM_ERR_INVALID_ARGUMENT;
            auto& s=*static_cast<State*>(context);if(s.writes.size()+length>65536)return MNM_ERR_INVALID_STATE;
            // Prepare all allocations before modifying the authoritative memory.
            std::vector<Write> writes;writes.reserve(length);auto bytes=static_cast<const uint8_t*>(source);
            for(uint32_t j=0;j<length;++j)writes.push_back(Write{address+j,{bytes[j]}});
            s.writes.reserve(s.writes.size()+length);size_t i,n;if(!location(s,address,length,2u,i,n))return MNM_ERR_INVALID_STATE;
            auto& target=i==SIZE_MAX?s.memory:s.regions[i].bytes;
            for(uint32_t j=0;j<length;++j) {target[n+j]=bytes[j];s.writes.push_back(std::move(writes[j]));}return MNM_OK;
        } catch(...) { return MNM_ERR_RUNTIME; }
    }
    static bool fail(Result& r,uint32_t pc,const char* reason) { r.exit=4;r.address=pc;r.reason=reason;return false; }
    bool invoke(Result& r,uint32_t pc,uint32_t handler) {
        if(handler<1 || handler>2)return fail(r,pc,"UNRESOLVED_IMPORT");
        if(handler==1) {
            // THREAD_MAPPING
        }
        const char* name=handler==1?"OSGetCurrentThread":"OSIsDebuggerInitialized";
        mnm_capability cap{sizeof(mnm_capability),MNM_SVC_NOT_IMPLEMENTED,MNM_VAL_NOT_IMPLEMENTED,0};
        uint32_t count=0;if(mnm_service_count(runtime,&count)!=MNM_OK || count>256)return fail(r,pc,"M10_CAPABILITY_MISMATCH");
        bool found=false;
        for(uint32_t index=0;index<count;++index) {
            char library[256]{},symbol[256]{};
            if(mnm_service_at(runtime,index,library,256,symbol,256,&cap)!=MNM_OK)return fail(r,pc,"M10_CAPABILITY_MISMATCH");
            if(std::strcmp(library,"coreinit")==0 && std::strcmp(symbol,name)==0) {found=true;break;}
        }
        if(!found || cap.status!=MNM_SVC_AVAILABLE || cap.validation!=MNM_VAL_MODEL_IMPLEMENTED)return fail(r,pc,"M10_CAPABILITY_MISMATCH");
        mnm_memory_callbacks memory{sizeof(mnm_memory_callbacks),1,&r.state,validate,read,write};
        mnm_service_call call{sizeof(mnm_service_call),nullptr,0,&memory};
        mnm_service_result result{sizeof(mnm_service_result),0,0};
        auto status=mnm_invoke(runtime,"coreinit",name,&call,&result);
        if(status!=MNM_OK)return fail(r,pc,handler==1?"CURRENT_THREAD_UNAVAILABLE":"DEBUGGER_STATE_UNAVAILABLE");
        if(result.has_return!=1)return fail(r,pc,"M9_RETURN_ABI_MISMATCH");
        r.state.gpr[3]=result.r3;return true;
    }
};
