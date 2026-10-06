// Woomy native ABI 2. Independent implementation; guest addresses are integers.
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>
#include <fcntl.h>
#include <io.h>
struct Write { uint32_t address; std::vector<uint8_t> bytes; };
struct Region { uint32_t base, permissions, kind; std::vector<uint8_t> provenance, bytes; };
struct CallEvent { uint32_t caller, site, target, kind, return_target; };
struct State {
    uint32_t gpr[32]{}; uint64_t fpr[32]{}; uint8_t cr[8]{};
    uint32_t lr{}, ctr{}, xer{}, fpscr{}, memory_base{};
    std::vector<uint8_t> memory;
    std::vector<Write> writes;
    std::vector<Region> regions;
    std::vector<CallEvent> calls;
};
struct Result { State state; std::vector<uint32_t> trace; uint32_t exit, address; std::string reason; };
static bool bounded(const State& s) {
    if(s.regions.size()>256u || s.calls.size()>65536u) return false;
    size_t total=0;
    for(size_t i=0;i<s.regions.size();++i) {
        const auto& r=s.regions[i]; total+=r.bytes.size();
        if(r.bytes.empty() || total>1048576u || r.kind>4u || (r.permissions & ~7u)!=0u || r.provenance.empty() || r.provenance.size()>4096u || uint64_t(r.base)+r.bytes.size()>0x100000000ull) return false;
        for(size_t j=0;j<i;++j) { const auto& b=s.regions[j]; if(uint64_t(r.base)<uint64_t(b.base)+b.bytes.size() && uint64_t(b.base)<uint64_t(r.base)+r.bytes.size()) return false; }
    }
    return s.memory.size() <= 1048576u && uint64_t(s.memory_base) + s.memory.size() <= 0x100000000ull && s.writes.size() <= 65536u;
}
[[maybe_unused]] static uint32_t rotate32(uint32_t v, uint32_t n) { return n == 0u ? v : (v << n) | (v >> (32u-n)); }
[[maybe_unused]] static uint32_t compare32(uint32_t a, uint32_t b, uint32_t xer, bool is_signed) {
    if(is_signed) {a^=0x80000000u; b^=0x80000000u;}
    return (a<b ? 8u : a>b ? 4u : 2u) | (xer >> 31u);
}
[[maybe_unused]] static uint32_t flags32(uint32_t a, uint32_t b, uint32_t v, uint32_t xer, bool sub, bool carry, bool overflow) {
    if(carry) { bool c=sub ? a>=b : uint64_t(a)+uint64_t(b)>0xffffffffull; xer=(xer & ~0x20000000u) | (uint32_t(c) << 29u); }
    if(overflow) {bool o=((sub ? (a^b) : ~(a^b)) & (a^v) & 0x80000000u)!=0u; xer=(xer & ~0x40000000u) | (uint32_t(o) << 30u) | (uint32_t(o) << 31u);}
    return xer;
}
[[maybe_unused]] static uint32_t branch_condition(State& s, uint32_t cr_bit, bool expected, bool decrement, bool zero) {
    if(decrement) s.ctr-=1u;
    const bool ctr_ok=!decrement || (s.ctr==0u)==zero;
    const bool cr_ok=cr_bit==32u || (((s.cr[cr_bit/4u] >> (3u-cr_bit%4u)) & 1u)!=0u)==expected;
    return uint32_t(ctr_ok && cr_ok);
}
static constexpr const char* memory_fault = "Unmapped or unaligned memory; guest exception semantics not implemented";
static Result fault(Result r, uint32_t pc, const char* why, uint32_t kind=2u) { r.exit=kind; r.address=pc; r.reason=why; return r; }
static bool location(const State& s, uint32_t address, uint32_t width, uint32_t permission, size_t& region, size_t& offset) {
    if(width==0u || uint64_t(address)+width>0x100000000ull) return false;
    if(s.regions.empty()) {
        if(address<s.memory_base || uint64_t(address-s.memory_base)+width>s.memory.size()) return false;
        region=SIZE_MAX; offset=address-s.memory_base; return true;
    }
    for(size_t i=0;i<s.regions.size();++i) {
        const auto& r=s.regions[i];
        if((r.permissions & permission)==permission && address>=r.base && uint64_t(address)+width<=uint64_t(r.base)+r.bytes.size()) { region=i; offset=address-r.base; return true; }
    }
    return false;
}
[[maybe_unused]] static bool load(const State& s, uint32_t address, uint64_t& result, uint32_t width=4u) {
    size_t i,n; if(address % (width<4u ? width:4u)!=0u || !location(s,address,width,1u,i,n)) return false;
    const auto& bytes=i==SIZE_MAX ? s.memory : s.regions[i].bytes;
    result=0u; for(uint32_t j=0;j<width;++j) result=(result<<8u)|bytes[n+j]; return true;
}
[[maybe_unused]] static bool store(State& s, uint32_t address, uint64_t value, uint32_t width=4u) {
    size_t i,n; if(address % (width<4u ? width:4u)!=0u || !location(s,address,width,2u,i,n) || s.writes.size()>=131072u) return false;
    auto& bytes=i==SIZE_MAX ? s.memory : s.regions[i].bytes;
    Write w{address,{}};
    for(uint32_t j=0;j<width;++j) { uint8_t b=uint8_t(value>>(8u*(width-1u-j))); bytes[n+j]=b; w.bytes.push_back(b); }
    s.writes.push_back(std::move(w)); return true;
}
[[maybe_unused]] static uint32_t mul_high_signed(uint32_t a, uint32_t b) {
    // Unsigned high product corrected for the two sign bits; no host signed cast/shift assumptions.
    uint32_t high=uint32_t((uint64_t(a)*b)>>32u);
    return high - ((a>>31u)!=0u ? b:0u) - ((b>>31u)!=0u ? a:0u);
}
struct WoomyRuntime {
    uint32_t outer, max_calls, max_depth, max_trace, calls=0;
    std::vector<uint32_t> frames, entries, blocks;
    std::vector<std::pair<uint32_t,uint32_t>> owners, imports;
    WoomyRuntime(uint32_t o,uint32_t c,uint32_t d,uint32_t t):outer(o),max_calls(c),max_depth(d),max_trace(t){}
    static bool contains(const std::vector<uint32_t>& values,uint32_t v) { for(auto x:values) if(x==v) return true; return false; }
    uint32_t owner(uint32_t pc) const { for(auto x:owners) if(x.first==pc) return x.second; return 0u; }
    bool fail(Result& r,uint32_t pc,const char* reason) { r.exit=std::string(reason)=="UNRESOLVED_CALL" ? 5u : std::string(reason)=="UNRESOLVED_IMPORT" ? 6u : std::string(reason)=="RUNTIME_COPY_LIMIT" ? 7u : 4u; r.address=pc; r.reason=reason; return false; }
    bool fetch(Result& r,uint32_t pc) {
        size_t i,n;
        if((pc & 3u)!=0u || r.state.regions.empty() || !location(r.state,pc,4u,4u,i,n)) return fail(r,pc,"NON_EXECUTABLE_GUEST_ADDRESS");
        return true;
    }
    bool event(Result& r,CallEvent e) {
        if(r.state.calls.size()>=max_trace) { r.exit=3u; return false; }
        r.state.calls.push_back(e); return true;
    }
    bool invoke(Result& r,uint32_t pc,uint32_t handler) {
        auto& s=r.state;
        if(handler==1u) s.gpr[3]+=s.gpr[4];
        else if(handler==3u) s.gpr[3]=0x574f4f4du;
        else if(handler==2u) {
            const uint32_t dst=s.gpr[3],src=s.gpr[4],count=s.gpr[5];
            if(count>4096u) return fail(r,pc,"RUNTIME_COPY_LIMIT");
            std::vector<uint8_t> bytes;
            for(uint32_t n=0;n<count;++n) {
                size_t i,o; uint64_t b;
                if(uint64_t(src)+n>0xffffffffull || uint64_t(dst)+n>0xffffffffull || !location(s,dst+n,1u,2u,i,o) || !load(s,src+n,b,1u)) return fail(r,pc,"GUEST_MEMORY_FAULT");
                bytes.push_back(uint8_t(b));
            }
            for(uint32_t n=0;n<count;++n) if(!store(s,dst+n,bytes[n],1u)) return fail(r,pc,"GUEST_MEMORY_FAULT");
        } else return fail(r,pc,"UNRESOLVED_IMPORT");
        return true;
    }
    bool call(Result& r,uint32_t pc,uint32_t target,uint32_t kind,uint32_t& next) {
        auto& s=r.state; s.lr=pc+4u;
        if(calls>=max_calls || frames.size()>=max_depth) { r.exit=3u; return false; }
        if(!event(r,{owner(pc),pc,target,kind,s.lr})) return false;
        ++calls;
        if(kind==2u) {
            uint32_t handler=0u; for(auto x:imports) if(x.first==pc) handler=x.second;
            if(!invoke(r,pc,handler)) return false;
            next=s.lr & ~3u; return true;
        }
        if(!contains(entries,target)) return fail(r,pc,"UNRESOLVED_CALL");
        if(!fetch(r,target)) return false;
        frames.push_back(s.lr); next=target; return true;
    }
    bool returning(Result& r,uint32_t pc,uint32_t& next) {
        const uint32_t target=r.state.lr & ~3u;
        if(!event(r,{owner(pc),pc,target,3u,target})) return false;
        if(!frames.empty() && frames.back()==target) frames.pop_back();
        else if(target==outer && frames.empty()) { r.exit=0u; r.address=target; return false; }
        if(!contains(blocks,target)) return fail(r,pc,"UNREGISTERED_RETURN_TARGET");
        if(!fetch(r,target)) return false;
        next=target; return true;
    }
};
