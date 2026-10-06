
// Binary little-endian transport; guest memory remains big-endian. No C ABI layout assumptions.
static uint64_t read_number(unsigned bytes) {
    uint64_t v=0;
    for (unsigned i=0;i<bytes;++i) { int c=std::getchar(); if(c==EOF) throw std::runtime_error("truncated input"); v|=uint64_t(uint8_t(c)) << (8u*i); }
    return v;
}
static uint32_t read32() { return uint32_t(read_number(4)); }
static void number(uint64_t v, unsigned bytes) { for(unsigned i=0;i<bytes;++i) if(std::putchar(int(uint8_t(v >> (8u*i))))==EOF) throw std::runtime_error("output error"); }
static std::vector<uint8_t> bytes_in(uint32_t max) {
    uint32_t n=read32(); if(n>max) throw std::runtime_error("input bound");
    std::vector<uint8_t> v(n); for(auto& b:v) b=uint8_t(read_number(1)); return v;
}
static void bytes_out(const std::vector<uint8_t>& v) { number(v.size(),4); for(auto b:v) number(b,1); }
static State state_in() {
    State s; for(auto& r:s.gpr) r=read32(); for(auto& r:s.fpr) r=read_number(8); for(auto& r:s.cr) r=uint8_t(read_number(1));
    s.lr=read32(); s.ctr=read32(); s.xer=read32(); s.fpscr=read32(); s.memory_base=read32(); s.memory=bytes_in(1048576u);
    uint32_t n=read32(); if(n>65536u) throw std::runtime_error("write bound");
    for(uint32_t i=0;i<n;++i) { uint32_t address=read32(); s.writes.push_back(Write{address,bytes_in(8u)}); }
    n=read32(); if(n>256u) throw std::runtime_error("region bound");
    for(uint32_t i=0;i<n;++i) {
        Region region; region.base=read32(); region.permissions=read32(); region.kind=read32(); region.provenance=bytes_in(4096u); region.bytes=bytes_in(1048576u); s.regions.push_back(std::move(region));
    }
    n=read32(); if(n>65536u) throw std::runtime_error("call trace bound");
    for(uint32_t i=0;i<n;++i) { CallEvent event; event.caller=read32(); event.site=read32(); event.target=read32(); event.kind=read32(); event.return_target=read32(); s.calls.push_back(event); }
    return s;
}
static void state_out(const State& s) {
    for(auto r:s.gpr) number(r,4); for(auto r:s.fpr) number(r,8); for(auto r:s.cr) number(r,1);
    number(s.lr,4); number(s.ctr,4); number(s.xer,4); number(s.fpscr,4); number(s.memory_base,4); bytes_out(s.memory); number(s.writes.size(),4);
    for(const auto& w:s.writes) { number(w.address,4); bytes_out(w.bytes); }
    number(s.regions.size(),4); for(const auto& region:s.regions) { number(region.base,4); number(region.permissions,4); number(region.kind,4); bytes_out(region.provenance); bytes_out(region.bytes); }
    number(s.calls.size(),4); for(const auto& e:s.calls) { number(e.caller,4); number(e.site,4); number(e.target,4); number(e.kind,4); number(e.return_target,4); }
}
int main() {
    try {
        if(_setmode(_fileno(stdin),_O_BINARY)==-1 || _setmode(_fileno(stdout),_O_BINARY)==-1) return 4;
        if(read32()!=0x574f4f32u) return 2;
        uint32_t count=read32(); if(count>4096u) return 2;
        number(0x574f4f32u,4); number(count,4);
        for(uint32_t i=0;i<count;++i) {
            uint32_t budget=read32(); Result r=run(state_in(),budget);
            state_out(r.state); number(r.trace.size(),4); for(auto pc:r.trace) number(pc,4);
            number(r.exit,4); number(r.address,4); number(r.reason.size(),4); for(auto c:r.reason) number(uint8_t(c),1);
        }
        if(std::getchar()!=EOF) return 2;
        return std::fflush(stdout)==0 ? 0 : 3;
    } catch(const std::exception& e) { std::fprintf(stderr,"Woomy native harness: %s\n",e.what()); return 3; }
}
