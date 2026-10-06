// Streaming bridge for the locally installed Sony DSEE HX DirectShow filter.
// This program is ours. It loads
// C:\Program Files (x86)\Sony\Music Center\Sony.Earth\OmgDseeHxFilter.ax
// and never copies or embeds that file.

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <dshow.h>
#include <fcntl.h>
#include <io.h>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <condition_variable>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

static const wchar_t* kFilterPath =
    L"C:\\Program Files (x86)\\Sony\\Music Center\\Sony.Earth\\OmgDseeHxFilter.ax";
static const DWORD kOutputHz = 96000;

enum MessageType : uint32_t { kConfig = 1, kData = 2, kEos = 3, kQuit = 4, kReady = 1, kErr = 4 };

struct HxConfig {
    DWORD mode, channels, codec, inFs, outFs, inBits, outBits, vbr, abr;
};
struct __declspec(uuid("6AF76DBE-8634-4FD3-93ED-982EDE1BFDD5")) IHx : IUnknown {
    virtual HRESULT STDMETHODCALLTYPE Configure(HxConfig) = 0;
    virtual HRESULT STDMETHODCALLTYPE Mode(DWORD) = 0;
    virtual HRESULT STDMETHODCALLTYPE Output(DWORD, DWORD) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetOutput(DWORD*, DWORD*) = 0;
    virtual HRESULT STDMETHODCALLTYPE Extra(DWORD) = 0;
};
struct ISampleGrabberCB : IUnknown {
    virtual HRESULT STDMETHODCALLTYPE SampleCB(double, IMediaSample*) = 0;
    virtual HRESULT STDMETHODCALLTYPE BufferCB(double, BYTE*, long) = 0;
};
struct __declspec(uuid("6B652FFF-11FE-4FCE-92AD-0266B5D7C78F")) ISampleGrabber : IUnknown {
    virtual HRESULT STDMETHODCALLTYPE SetOneShot(BOOL) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetMediaType(const AM_MEDIA_TYPE*) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetConnectedMediaType(AM_MEDIA_TYPE*) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetBufferSamples(BOOL) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetCurrentBuffer(long*, long*) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetCurrentSample(IMediaSample**) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetCallback(ISampleGrabberCB*, long) = 0;
};

static std::mutex g_stdout_mu;

static bool read_full(void* data, uint32_t size) {
    auto* bytes = static_cast<uint8_t*>(data);
    uint32_t got = 0;
    while (got < size) {
        const int n = _read(_fileno(stdin), bytes + got, static_cast<unsigned>(size - got));
        if (n <= 0) return false;
        got += static_cast<uint32_t>(n);
    }
    return true;
}

static bool write_full(const void* data, uint32_t size) {
    auto* bytes = static_cast<const uint8_t*>(data);
    uint32_t sent = 0;
    while (sent < size) {
        const int n = _write(_fileno(stdout), bytes + sent, static_cast<unsigned>(size - sent));
        if (n <= 0) return false;
        sent += static_cast<uint32_t>(n);
    }
    return true;
}

static bool write_message(uint32_t type, const void* payload, uint32_t size) {
    std::lock_guard<std::mutex> lock(g_stdout_mu);
    return write_full(&type, 4) && write_full(&size, 4) && (size == 0 || write_full(payload, size));
}

static void write_err(uint32_t code, const char* text) {
    const uint32_t len = static_cast<uint32_t>(std::strlen(text));
    std::vector<uint8_t> payload(8 + len);
    std::memcpy(payload.data(), &code, 4);
    std::memcpy(payload.data() + 4, &len, 4);
    std::memcpy(payload.data() + 8, text, len);
    write_message(kErr, payload.data(), static_cast<uint32_t>(payload.size()));
    std::fflush(stdout);
}

struct Chunk {
    bool eos = false;
    std::vector<uint8_t> bytes;
};

static std::mutex g_queue_mu;
static std::condition_variable g_queue_cv;
static std::deque<Chunk> g_queue;
static bool g_stop = false;

static void push_chunk(Chunk chunk) {
    std::lock_guard<std::mutex> lock(g_queue_mu);
    g_queue.push_back(std::move(chunk));
    g_queue_cv.notify_one();
}

static bool pop_chunk(Chunk& out) {
    std::unique_lock<std::mutex> lock(g_queue_mu);
    g_queue_cv.wait(lock, [] { return g_stop || !g_queue.empty(); });
    if (g_queue.empty()) return false;
    out = std::move(g_queue.front());
    g_queue.pop_front();
    return true;
}

class PcmPin;
class PcmFilter : public IBaseFilter {
public:
    PcmFilter();
    ~PcmFilter();
    STDMETHODIMP QueryInterface(REFIID riid, void** out) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;
    STDMETHODIMP GetClassID(CLSID* id) override;
    STDMETHODIMP Stop() override;
    STDMETHODIMP Pause() override;
    STDMETHODIMP Run(REFERENCE_TIME start) override;
    STDMETHODIMP GetState(DWORD, FILTER_STATE* state) override;
    STDMETHODIMP SetSyncSource(IReferenceClock* clock) override;
    STDMETHODIMP GetSyncSource(IReferenceClock** clock) override;
    STDMETHODIMP EnumPins(IEnumPins** pins) override;
    STDMETHODIMP FindPin(LPCWSTR id, IPin** pin) override;
    STDMETHODIMP QueryFilterInfo(FILTER_INFO* info) override;
    STDMETHODIMP JoinFilterGraph(IFilterGraph* graph, LPCWSTR name) override;
    STDMETHODIMP QueryVendorInfo(LPWSTR* vendor) override;
    HRESULT Configure(const WAVEFORMATEX& format);
    PcmPin* pin = nullptr;
    FILTER_STATE state = State_Stopped;
    IFilterGraph* graph = nullptr;
    IReferenceClock* clock = nullptr;
    wchar_t name[128] = L"PCM";
    LONG refs = 1;
    std::thread worker;
};

class PcmPin : public IPin {
public:
    explicit PcmPin(PcmFilter* owner) : filter(owner) {}
    STDMETHODIMP QueryInterface(REFIID riid, void** out) override;
    STDMETHODIMP_(ULONG) AddRef() override { return filter->AddRef(); }
    STDMETHODIMP_(ULONG) Release() override { return filter->Release(); }
    STDMETHODIMP Connect(IPin* receive, const AM_MEDIA_TYPE* proposed) override;
    STDMETHODIMP ReceiveConnection(IPin*, const AM_MEDIA_TYPE*) override { return E_UNEXPECTED; }
    STDMETHODIMP Disconnect() override;
    STDMETHODIMP ConnectedTo(IPin** pin) override;
    STDMETHODIMP ConnectionMediaType(AM_MEDIA_TYPE* type) override;
    STDMETHODIMP QueryPinInfo(PIN_INFO* info) override;
    STDMETHODIMP QueryDirection(PIN_DIRECTION* dir) override { *dir = PINDIR_OUTPUT; return S_OK; }
    STDMETHODIMP QueryId(LPWSTR* id) override;
    STDMETHODIMP QueryAccept(const AM_MEDIA_TYPE* type) override;
    STDMETHODIMP EnumMediaTypes(IEnumMediaTypes** types) override;
    STDMETHODIMP QueryInternalConnections(IPin**, ULONG*) override { return E_NOTIMPL; }
    STDMETHODIMP EndOfStream() override { return S_OK; }
    STDMETHODIMP BeginFlush() override { flushing = true; return S_OK; }
    STDMETHODIMP EndFlush() override { flushing = false; return S_OK; }
    STDMETHODIMP NewSegment(REFERENCE_TIME, REFERENCE_TIME, double) override { return S_OK; }
    HRESULT Active();
    void Inactive();
    void DeliverLoop();
    PcmFilter* filter;
    IPin* peer = nullptr;
    IMemInputPin* input = nullptr;
    IMemAllocator* allocator = nullptr;
    WAVEFORMATEX format{};
    bool connected = false;
    bool flushing = false;
    uint64_t frames = 0;
};

class PinEnumerator : public IEnumPins {
public:
    PinEnumerator(PcmPin* pin, int index) : pin(pin), index(index) { pin->AddRef(); }
    ~PinEnumerator() { pin->Release(); }
    STDMETHODIMP QueryInterface(REFIID riid, void** out) override {
        if (riid == IID_IUnknown || riid == IID_IEnumPins) { *out = this; AddRef(); return S_OK; }
        *out = nullptr; return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refs); }
    STDMETHODIMP_(ULONG) Release() override {
        const ULONG left = InterlockedDecrement(&refs);
        if (!left) delete this;
        return left;
    }
    STDMETHODIMP Next(ULONG count, IPin** pins, ULONG* fetched) override {
        ULONG wrote = 0;
        if (index == 0 && count > 0) { pins[0] = pin; pin->AddRef(); index = 1; wrote = 1; }
        if (fetched) *fetched = wrote;
        return wrote == count ? S_OK : S_FALSE;
    }
    STDMETHODIMP Skip(ULONG count) override { index += static_cast<int>(count); return S_OK; }
    STDMETHODIMP Reset() override { index = 0; return S_OK; }
    STDMETHODIMP Clone(IEnumPins** clone) override { *clone = new PinEnumerator(pin, index); return S_OK; }
    PcmPin* pin;
    int index;
    LONG refs = 1;
};

class MediaTypeEnumerator : public IEnumMediaTypes {
public:
    MediaTypeEnumerator(const WAVEFORMATEX& format, int index) : format(format), index(index) {}
    STDMETHODIMP QueryInterface(REFIID riid, void** out) override {
        if (riid == IID_IUnknown || riid == IID_IEnumMediaTypes) { *out = this; AddRef(); return S_OK; }
        *out = nullptr; return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refs); }
    STDMETHODIMP_(ULONG) Release() override {
        const ULONG left = InterlockedDecrement(&refs);
        if (!left) delete this;
        return left;
    }
    STDMETHODIMP Next(ULONG count, AM_MEDIA_TYPE** types, ULONG* fetched) override;
    STDMETHODIMP Skip(ULONG count) override { index += static_cast<int>(count); return S_OK; }
    STDMETHODIMP Reset() override { index = 0; return S_OK; }
    STDMETHODIMP Clone(IEnumMediaTypes** clone) override { *clone = new MediaTypeEnumerator(format, index); return S_OK; }
    WAVEFORMATEX format;
    int index;
    LONG refs = 1;
};

static AM_MEDIA_TYPE media_type(const WAVEFORMATEX& format) {
    AM_MEDIA_TYPE type{};
    type.majortype = MEDIATYPE_Audio;
    type.subtype = MEDIASUBTYPE_PCM;
    type.bFixedSizeSamples = TRUE;
    type.lSampleSize = format.nBlockAlign;
    type.formattype = FORMAT_WaveFormatEx;
    type.cbFormat = sizeof(WAVEFORMATEX);
    return type;
}

HRESULT MediaTypeEnumerator::Next(ULONG count, AM_MEDIA_TYPE** types, ULONG* fetched) {
    ULONG wrote = 0;
    if (index == 0 && count > 0) {
        auto* type = static_cast<AM_MEDIA_TYPE*>(CoTaskMemAlloc(sizeof(AM_MEDIA_TYPE)));
        if (!type) return E_OUTOFMEMORY;
        *type = media_type(format);
        type->pbFormat = static_cast<BYTE*>(CoTaskMemAlloc(sizeof(WAVEFORMATEX)));
        if (!type->pbFormat) { CoTaskMemFree(type); return E_OUTOFMEMORY; }
        std::memcpy(type->pbFormat, &format, sizeof(WAVEFORMATEX));
        types[0] = type;
        index = 1;
        wrote = 1;
    }
    if (fetched) *fetched = wrote;
    return wrote == count ? S_OK : S_FALSE;
}

PcmFilter::PcmFilter() { pin = new PcmPin(this); }
PcmFilter::~PcmFilter() {
    if (pin) {
        pin->Inactive();
        pin->Disconnect();
        delete pin;
        pin = nullptr;
    }
    if (clock) clock->Release();
}

STDMETHODIMP PcmFilter::QueryInterface(REFIID riid, void** out) {
    if (riid == IID_IUnknown || riid == IID_IBaseFilter || riid == IID_IMediaFilter || riid == IID_IPersist) {
        *out = static_cast<IBaseFilter*>(this);
        AddRef();
        return S_OK;
    }
    *out = nullptr;
    return E_NOINTERFACE;
}
STDMETHODIMP_(ULONG) PcmFilter::AddRef() { return InterlockedIncrement(&refs); }
STDMETHODIMP_(ULONG) PcmFilter::Release() {
    const ULONG left = InterlockedDecrement(&refs);
    if (!left) delete this;
    return left;
}
STDMETHODIMP PcmFilter::GetClassID(CLSID* id) {
    *id = CLSID{0x8a1e0b21, 0x7c3d, 0x4e55, {0x9b, 0x0a, 0x6e, 0x5d, 0x4c, 0x3b, 0x2a, 0x19}};
    return S_OK;
}
STDMETHODIMP PcmFilter::Stop() {
    state = State_Stopped;
    pin->Inactive();
    return S_OK;
}
STDMETHODIMP PcmFilter::Pause() {
    state = State_Paused;
    return pin->Active();
}
STDMETHODIMP PcmFilter::Run(REFERENCE_TIME) {
    const HRESULT hr = pin->Active();
    state = State_Running;
    g_queue_cv.notify_all();
    return hr;
}
STDMETHODIMP PcmFilter::GetState(DWORD, FILTER_STATE* out) { *out = state; return S_OK; }
STDMETHODIMP PcmFilter::SetSyncSource(IReferenceClock* next) {
    if (clock) clock->Release();
    clock = next;
    if (clock) clock->AddRef();
    return S_OK;
}
STDMETHODIMP PcmFilter::GetSyncSource(IReferenceClock** out) {
    *out = clock;
    if (clock) clock->AddRef();
    return S_OK;
}
STDMETHODIMP PcmFilter::EnumPins(IEnumPins** pins) { *pins = new PinEnumerator(pin, 0); return S_OK; }
STDMETHODIMP PcmFilter::FindPin(LPCWSTR id, IPin** out) {
    if (id && wcscmp(id, L"Output") == 0) { *out = pin; pin->AddRef(); return S_OK; }
    *out = nullptr;
    return VFW_E_NOT_FOUND;
}
STDMETHODIMP PcmFilter::QueryFilterInfo(FILTER_INFO* info) {
    info->pGraph = graph;
    if (graph) graph->AddRef();
    wcsncpy_s(info->achName, 128, name, _TRUNCATE);
    return S_OK;
}
STDMETHODIMP PcmFilter::JoinFilterGraph(IFilterGraph* next, LPCWSTR nextName) {
    graph = next;
    if (nextName) wcsncpy_s(name, 128, nextName, _TRUNCATE);
    return S_OK;
}
STDMETHODIMP PcmFilter::QueryVendorInfo(LPWSTR* vendor) { *vendor = nullptr; return E_NOTIMPL; }
HRESULT PcmFilter::Configure(const WAVEFORMATEX& format) { pin->format = format; return S_OK; }

STDMETHODIMP PcmPin::QueryInterface(REFIID riid, void** out) {
    if (riid == IID_IUnknown || riid == IID_IPin) { *out = static_cast<IPin*>(this); AddRef(); return S_OK; }
    *out = nullptr;
    return E_NOINTERFACE;
}
STDMETHODIMP PcmPin::QueryAccept(const AM_MEDIA_TYPE* type) {
    if (!type || type->majortype != MEDIATYPE_Audio || type->subtype != MEDIASUBTYPE_PCM) return S_FALSE;
    if (type->formattype != FORMAT_WaveFormatEx || !type->pbFormat) return S_FALSE;
    auto* wf = reinterpret_cast<const WAVEFORMATEX*>(type->pbFormat);
    if (wf->nSamplesPerSec != format.nSamplesPerSec || wf->wBitsPerSample != format.wBitsPerSample || wf->nChannels != format.nChannels) {
        return S_FALSE;
    }
    return S_OK;
}
STDMETHODIMP PcmPin::EnumMediaTypes(IEnumMediaTypes** types) { *types = new MediaTypeEnumerator(format, 0); return S_OK; }
STDMETHODIMP PcmPin::QueryId(LPWSTR* id) {
    const size_t bytes = 7 * sizeof(wchar_t);
    *id = static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if (!*id) return E_OUTOFMEMORY;
    std::memcpy(*id, L"Output", bytes);
    return S_OK;
}
STDMETHODIMP PcmPin::QueryPinInfo(PIN_INFO* info) {
    info->pFilter = filter;
    filter->AddRef();
    info->dir = PINDIR_OUTPUT;
    wcsncpy_s(info->achName, 128, L"Output", _TRUNCATE);
    return S_OK;
}
STDMETHODIMP PcmPin::ConnectedTo(IPin** out) {
    if (!peer) { *out = nullptr; return VFW_E_NOT_CONNECTED; }
    *out = peer;
    peer->AddRef();
    return S_OK;
}
STDMETHODIMP PcmPin::ConnectionMediaType(AM_MEDIA_TYPE* type) {
    if (!connected) return VFW_E_NOT_CONNECTED;
    *type = media_type(format);
    type->pbFormat = static_cast<BYTE*>(CoTaskMemAlloc(sizeof(WAVEFORMATEX)));
    if (!type->pbFormat) return E_OUTOFMEMORY;
    std::memcpy(type->pbFormat, &format, sizeof(WAVEFORMATEX));
    return S_OK;
}
STDMETHODIMP PcmPin::Disconnect() {
    if (!connected) return S_FALSE;
    if (input) { input->Release(); input = nullptr; }
    if (allocator) { allocator->Release(); allocator = nullptr; }
    if (peer) { peer->Release(); peer = nullptr; }
    connected = false;
    return S_OK;
}
STDMETHODIMP PcmPin::Connect(IPin* receive, const AM_MEDIA_TYPE* proposed) {
    if (connected) return VFW_E_ALREADY_CONNECTED;
    AM_MEDIA_TYPE type = media_type(format);
    type.pbFormat = reinterpret_cast<BYTE*>(&format);
    if (proposed && QueryAccept(proposed) != S_OK) return VFW_E_TYPE_NOT_ACCEPTED;
    if (receive->QueryAccept(&type) != S_OK) return VFW_E_TYPE_NOT_ACCEPTED;
    HRESULT hr = receive->ReceiveConnection(this, &type);
    if (FAILED(hr)) return hr;
    hr = receive->QueryInterface(IID_IMemInputPin, reinterpret_cast<void**>(&input));
    if (FAILED(hr)) { receive->Disconnect(); return hr; }
    hr = input->GetAllocator(&allocator);
    if (FAILED(hr)) {
        hr = CoCreateInstance(CLSID_MemoryAllocator, nullptr, CLSCTX_INPROC_SERVER, IID_IMemAllocator, reinterpret_cast<void**>(&allocator));
    }
    if (FAILED(hr)) { Disconnect(); receive->Disconnect(); return hr; }
    ALLOCATOR_PROPERTIES props{};
    props.cBuffers = 8;
    props.cbBuffer = 4096;
    props.cbAlign = 1;
    ALLOCATOR_PROPERTIES actual{};
    hr = allocator->SetProperties(&props, &actual);
    if (FAILED(hr)) { Disconnect(); receive->Disconnect(); return hr; }
    hr = input->NotifyAllocator(allocator, FALSE);
    if (FAILED(hr)) { Disconnect(); receive->Disconnect(); return hr; }
    peer = receive;
    peer->AddRef();
    connected = true;
    return S_OK;
}

HRESULT PcmPin::Active() {
    if (!allocator) return VFW_E_NOT_CONNECTED;
    const HRESULT hr = allocator->Commit();
    if (FAILED(hr)) return hr;
    if (!filter->worker.joinable()) {
        filter->worker = std::thread([this] {
            CoInitializeEx(nullptr, COINIT_MULTITHREADED);
            DeliverLoop();
            CoUninitialize();
        });
    }
    return S_OK;
}
void PcmPin::Inactive() {
    {
        std::lock_guard<std::mutex> lock(g_queue_mu);
        g_stop = true;
    }
    g_queue_cv.notify_all();
    if (peer) peer->BeginFlush();
    if (filter->worker.joinable()) filter->worker.join();
    if (peer) peer->EndFlush();
    if (allocator) allocator->Decommit();
    g_stop = false;
}

void PcmPin::DeliverLoop() {
    while (true) {
        Chunk chunk;
        if (!pop_chunk(chunk)) return;
        if (chunk.eos) {
            if (peer && !flushing) peer->EndOfStream();
            write_message(kEos, nullptr, 0);
            std::fflush(stdout);
            return;
        }
        size_t offset = 0;
        while (offset < chunk.bytes.size() && !flushing) {
            IMediaSample* sample = nullptr;
            const HRESULT bufferHr = allocator->GetBuffer(&sample, nullptr, nullptr, 0);
            if (FAILED(bufferHr) || !sample) return;
            BYTE* data = nullptr;
            sample->GetPointer(&data);
            const long capacity = sample->GetSize();
            const size_t room = capacity > 0 ? static_cast<size_t>(capacity) : 0;
            const size_t align = format.nBlockAlign ? format.nBlockAlign : 4;
            size_t take = chunk.bytes.size() - offset;
            if (take > room) take = room - (room % align);
            if (take == 0) {
                sample->Release();
                return;
            }
            std::memcpy(data, chunk.bytes.data() + offset, take);
            sample->SetActualDataLength(static_cast<long>(take));
            const uint64_t frameCount = take / align;
            const REFERENCE_TIME t0 = static_cast<REFERENCE_TIME>(frames * 10000000ULL / format.nSamplesPerSec);
            frames += frameCount;
            const REFERENCE_TIME t1 = static_cast<REFERENCE_TIME>(frames * 10000000ULL / format.nSamplesPerSec);
            sample->SetTime(const_cast<REFERENCE_TIME*>(&t0), const_cast<REFERENCE_TIME*>(&t1));
            sample->SetSyncPoint(TRUE);
            const HRESULT received = input->Receive(sample);
            sample->Release();
            if (FAILED(received)) return;
            offset += take;
        }
    }
}

class Capture : public ISampleGrabberCB {
public:
    STDMETHODIMP QueryInterface(REFIID, void** out) override { *out = static_cast<ISampleGrabberCB*>(this); return S_OK; }
    STDMETHODIMP_(ULONG) AddRef() override { return 2; }
    STDMETHODIMP_(ULONG) Release() override { return 1; }
    STDMETHODIMP SampleCB(double, IMediaSample* sample) override {
        BYTE* data = nullptr;
        if (FAILED(sample->GetPointer(&data))) return E_FAIL;
        const long size = sample->GetActualDataLength();
        if (size < 0) return E_FAIL;
        return write_message(kData, data, static_cast<uint32_t>(size)) ? S_OK : E_FAIL;
    }
    STDMETHODIMP BufferCB(double, BYTE*, long) override { return E_NOTIMPL; }
};

static IPin* first_pin(IBaseFilter* filter, PIN_DIRECTION direction) {
    IEnumPins* pins = nullptr;
    if (FAILED(filter->EnumPins(&pins))) return nullptr;
    IPin* pin = nullptr;
    while (pins->Next(1, &pin, nullptr) == S_OK) {
        PIN_DIRECTION current;
        pin->QueryDirection(&current);
        if (current == direction) { pins->Release(); return pin; }
        pin->Release();
    }
    pins->Release();
    return nullptr;
}

static int run_graph(uint32_t channels, uint32_t codec, uint32_t inHz, uint32_t outHz, uint32_t inBits, uint32_t outBits, uint32_t vbr, uint32_t abr) {
    if (GetFileAttributesW(kFilterPath) == INVALID_FILE_ATTRIBUTES) {
        write_err(1, "missing");
        return 3;
    }
    SetDllDirectoryW(L"C:\\Program Files (x86)\\Sony\\Music Center\\Sony.Earth");
    HMODULE module = LoadLibraryW(kFilterPath);
    if (!module) { write_err(2, "LoadLibrary failed"); return 3; }
    GUID cls{0xa481d160, 0xab4d, 0x4b97, {0x93, 0xfe, 0xc1, 0xf4, 0xb4, 0xb7, 0x81, 0xdf}};
    auto get = reinterpret_cast<HRESULT(STDAPICALLTYPE*)(REFCLSID, REFIID, void**)>(GetProcAddress(module, "DllGetClassObject"));
    if (!get) { write_err(2, "DllGetClassObject missing"); return 3; }
    IClassFactory* factory = nullptr;
    if (FAILED(get(cls, IID_IClassFactory, reinterpret_cast<void**>(&factory)))) { write_err(2, "class factory failed"); return 3; }
    IBaseFilter* hx = nullptr;
    HRESULT hr = factory->CreateInstance(nullptr, IID_IBaseFilter, reinterpret_cast<void**>(&hx));
    factory->Release();
    if (FAILED(hr)) { write_err(2, "HX instance failed"); return 3; }
    IHx* settings = nullptr;
    hr = hx->QueryInterface(__uuidof(IHx), reinterpret_cast<void**>(&settings));
    if (FAILED(hr)) { hx->Release(); write_err(3, "HX settings interface missing"); return 3; }
    HxConfig config{1, channels, codec, inHz, outHz, inBits, outBits, vbr, abr};
    hr = settings->Configure(config);
    settings->Release();
    if (FAILED(hr)) { hx->Release(); write_err(3, "Configure failed"); return 3; }

    IGraphBuilder* graph = nullptr;
    hr = CoCreateInstance(CLSID_FilterGraph, nullptr, CLSCTX_INPROC_SERVER, IID_IGraphBuilder, reinterpret_cast<void**>(&graph));
    if (FAILED(hr)) { hx->Release(); write_err(4, "graph failed"); return 3; }
    auto* source = new PcmFilter();
    WAVEFORMATEX wf{};
    wf.wFormatTag = WAVE_FORMAT_PCM;
    wf.nChannels = static_cast<WORD>(channels);
    wf.nSamplesPerSec = inHz;
    wf.wBitsPerSample = static_cast<WORD>(inBits);
    wf.nBlockAlign = static_cast<WORD>(wf.nChannels * wf.wBitsPerSample / 8);
    wf.nAvgBytesPerSec = wf.nSamplesPerSec * wf.nBlockAlign;
    source->Configure(wf);
    graph->AddFilter(source, L"PCM");
    source->Release();
    graph->AddFilter(hx, L"Sony DSEE HX");
    GUID grabCls{0xc1f400a0, 0x3f08, 0x11d3, {0x9f, 0x0b, 0x00, 0x60, 0x08, 0x03, 0x9e, 0x37}};
    IBaseFilter* grab = nullptr;
    hr = CoCreateInstance(grabCls, nullptr, CLSCTX_INPROC_SERVER, IID_IBaseFilter, reinterpret_cast<void**>(&grab));
    if (FAILED(hr)) { graph->Release(); hx->Release(); write_err(4, "grabber failed"); return 3; }
    ISampleGrabber* grabber = nullptr;
    grab->QueryInterface(__uuidof(ISampleGrabber), reinterpret_cast<void**>(&grabber));
    AM_MEDIA_TYPE want{};
    want.majortype = MEDIATYPE_Audio;
    want.subtype = MEDIASUBTYPE_PCM;
    want.formattype = FORMAT_WaveFormatEx;
    grabber->SetMediaType(&want);
    graph->AddFilter(grab, L"Capture");
    GUID nullCls{0xc1f400a4, 0x3f08, 0x11d3, {0x9f, 0x0b, 0x00, 0x60, 0x08, 0x03, 0x9e, 0x37}};
    IBaseFilter* sink = nullptr;
    hr = CoCreateInstance(nullCls, nullptr, CLSCTX_INPROC_SERVER, IID_IBaseFilter, reinterpret_cast<void**>(&sink));
    if (FAILED(hr)) { graph->Release(); hx->Release(); grab->Release(); write_err(4, "sink failed"); return 3; }
    graph->AddFilter(sink, L"Sink");
    IPin* sourceOut = first_pin(source, PINDIR_OUTPUT);
    IPin* hxIn = first_pin(hx, PINDIR_INPUT);
    IPin* hxOut = first_pin(hx, PINDIR_OUTPUT);
    IPin* grabIn = first_pin(grab, PINDIR_INPUT);
    IPin* grabOut = first_pin(grab, PINDIR_OUTPUT);
    IPin* sinkIn = first_pin(sink, PINDIR_INPUT);
    if (!sourceOut || !hxIn || !hxOut || !grabIn || !grabOut || !sinkIn) {
        write_err(4, "pin missing");
        graph->Release(); hx->Release(); grab->Release(); sink->Release();
        return 3;
    }
    hr = graph->Connect(sourceOut, hxIn);
    if (FAILED(hr)) { write_err(4, "input connect failed"); goto done; }
    hr = graph->ConnectDirect(hxOut, grabIn, nullptr);
    if (FAILED(hr)) { write_err(4, "output connect failed"); goto done; }
    hr = graph->ConnectDirect(grabOut, sinkIn, nullptr);
    if (FAILED(hr)) { write_err(4, "sink connect failed"); goto done; }
    {
        AM_MEDIA_TYPE actual{};
        hr = grabber->GetConnectedMediaType(&actual);
        auto* outWf = reinterpret_cast<WAVEFORMATEX*>(actual.pbFormat);
        if (FAILED(hr) || !outWf) {
            write_err(4, "output format missing");
            goto done;
        }
        if (outWf->wBitsPerSample != 24 || outWf->nChannels != 2 || outWf->nSamplesPerSec == 0) {
            if (actual.pbFormat) CoTaskMemFree(actual.pbFormat);
            write_err(4, "unexpected output format");
            goto done;
        }
        if (actual.pbFormat) CoTaskMemFree(actual.pbFormat);
        Capture callback;
        grabber->SetCallback(&callback, 0);
        IMediaFilter* media = nullptr;
        graph->QueryInterface(IID_IMediaFilter, reinterpret_cast<void**>(&media));
        media->SetSyncSource(nullptr);
        media->Release();
        IMediaControl* control = nullptr;
        graph->QueryInterface(IID_IMediaControl, reinterpret_cast<void**>(&control));
        hr = control->Run();
        if (FAILED(hr)) { control->Release(); write_err(4, "graph run failed"); goto done; }
        uint32_t ready[3] = {outWf->nSamplesPerSec, outWf->wBitsPerSample, static_cast<uint32_t>(outWf->nChannels)};
        write_message(kReady, ready, sizeof(ready));
        std::fflush(stdout);
        bool sawEos = false;
        while (true) {
            uint32_t type = 0, size = 0;
            if (!read_full(&type, 4) || !read_full(&size, 4) || size > 1024 * 1024) break;
            std::vector<uint8_t> payload(size);
            if (size && !read_full(payload.data(), size)) break;
            if (type == kQuit) break;
            if (type == kEos) {
                push_chunk(Chunk{true, {}});
                sawEos = true;
                break;
            }
            if (type == kData) push_chunk(Chunk{false, std::move(payload)});
        }
        if (!sawEos) {
            std::lock_guard<std::mutex> lock(g_queue_mu);
            g_stop = true;
            g_queue_cv.notify_all();
        }
        if (source->worker.joinable()) source->worker.join();
        control->Stop();
        control->Release();
    }
done:
    if (sourceOut) sourceOut->Release();
    if (hxIn) hxIn->Release();
    if (hxOut) hxOut->Release();
    if (grabIn) grabIn->Release();
    if (grabOut) grabOut->Release();
    if (sinkIn) sinkIn->Release();
    grabber->Release();
    grab->Release();
    sink->Release();
    hx->Release();
    graph->Release();
    return 0;
}

int wmain() {
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
    if (FAILED(CoInitializeEx(nullptr, COINIT_MULTITHREADED))) return 1;
    uint32_t type = 0, size = 0;
    if (!read_full(&type, 4) || !read_full(&size, 4) || type != kConfig || size != 32) {
        write_err(4, "expected config");
        CoUninitialize();
        return 2;
    }
    uint32_t fields[8]{};
    if (!read_full(fields, sizeof(fields))) { CoUninitialize(); return 2; }
    const int code = run_graph(fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6], fields[7]);
    CoUninitialize();
    return code;
}
