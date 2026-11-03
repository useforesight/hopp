// hopp-share-clean: leave overlay layers marked no_screen_share out of screen shares,
// instead of Hyprland's black boxes.
//
// Hyprland shares a monitor by copying each finished frame into a "mirror" buffer, then
// painting a black box over every no_screen_share surface. A full-screen overlay (Hopp's
// marks layer, the draw pad) turns the whole share black. This plugin, for overlay-level
// layers only (nothing but the cursor, notifications and the lock screen draws above
// them):
//
//   1. At the start of the frame, takes those layers out of the monitor's layer list.
//   2. At the last moment, puts them back and queues: a snapshot of the frame so far into
//      the mirror buffer, the layers themselves, then a marker that tells Hyprland the
//      mirror is already done (so its own copy, with the layers in, is skipped).
//   3. Switches the black box off for those layers while the mirror is clean.
//
// Any frame it can't do that for (zoom, lock screen, HDR copy, a fullscreen game) falls
// back to Hyprland's black boxes, and the mirror is refreshed in full before it trusts
// it again. No function hooks (Hyprland's don't work on ARM): events and a pass element.
//
// Built against the exact Hyprland it runs in; refuses to load into any other.

// Everything outside Hyprland first: the access trick below must only touch its headers.
#include <algorithm>
#include <any>
#include <array>
#include <chrono>
#include <deque>
#include <expected>
#include <filesystem>
#include <format>
#include <fstream>
#include <functional>
#include <iostream>
#include <list>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <ranges>
#include <regex>
#include <set>
#include <span>
#include <sstream>
#include <string>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <variant>
#include <vector>
#include <hyprutils/memory/SharedPtr.hpp>
#include <hyprutils/memory/WeakPtr.hpp>
#include <hyprutils/memory/UniquePtr.hpp>
#include <hyprutils/math/Region.hpp>
#include <hyprutils/math/Box.hpp>
#include <hyprutils/math/Vector2D.hpp>
#include <hyprutils/signal/Signal.hpp>
#include <hyprutils/animation/AnimatedVariable.hpp>
#include <hyprlang.hpp>
#include <wayland-server.h>

#define private public
#define protected public
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <hyprland/src/render/pass/PassElement.hpp>
#include <hyprland/src/desktop/view/LayerSurface.hpp>
#include <hyprland/src/desktop/state/LayerState.hpp>
#include <hyprland/src/managers/SessionLockManager.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/output/MonitorResources.hpp>
#include <hyprland/src/helpers/time/Time.hpp>
#undef private
#undef protected

static HANDLE PHANDLE = nullptr;

#ifdef HOPP_SHARE_CLEAN_BENCH
#include "bench.hpp"
#else
// The production build carries no measuring code.
namespace bench {
    constexpr bool forceNoSimplify() {
        return false;
    }
    constexpr bool holdNothing() {
        return false;
    }
    inline void frameBegin() {}
    inline void frameEnd() {}
    inline void snapshotBegin() {}
    inline void snapshotEnd() {}
    struct SCpuTimer {};
}
#endif

static Hyprutils::Signal::CHyprSignalListener g_stageListener;

constexpr size_t OVERLAY  = 3; // ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY
constexpr auto OVERRIDE = Desktop::Types::PRIORITY_SET_PROP;

struct SMonitorState {
    // The mirror buffer holds no pixels of a hidden layer.
    bool clean = false;
    // This frame: the layers taken out, and the full list to put back.
    std::vector<PHLLSREF> held;
    std::vector<PHLLSREF> original;
    bool                  active = false;
    // A held layer has opaque pixels, so it could hide what's under it from the snapshot.
    bool mayOcclude = false;
};

static std::unordered_map<MONITORID, SMonitorState> g_monitors;

// The rule as configured, without this plugin's override.
static bool hiddenByRule(const PHLLS& ls) {
    auto& rule = ls->m_ruleApplicator->noScreenShare();
    rule.unset(OVERRIDE);
    return rule.valueOrDefault();
}

// Whether the layer declares opaque pixels. Hyprland skips drawing whatever an opaque
// surface covers; drawn after the snapshot, such a layer would leave holes in it.
static bool opaque(const PHLLS& ls) {
    const auto view = ls->wlSurface();
    const auto surf = view ? view->resource() : nullptr;
    if (!surf)
        return false;
    return !surf->m_current.opaque.empty() || (surf->m_current.texture && surf->m_current.texture->m_opaque);
}

static bool fullFrame(const PHLMONITOR& mon) {
    CRegion rest{CBox{{}, mon->m_transformedSize}};
    rest.subtract(g_pHyprRenderer->m_renderData.finalDamage);
    return rest.empty();
}

// Copies the frame drawn so far into the mirror buffer, as Hyprland's end-of-frame copy
// would, then marks the mirror clean (or due a full refresh, if it wasn't before).
class CSnapshotElement : public IPassElement {
  public:
    CSnapshotElement(PHLMONITOR mon, bool occlude) : m_mon(mon), m_occlude(occlude) {}

    std::vector<UP<IPassElement>> draw() override {
        const auto mon = m_mon.lock();
        if (!mon || mon != g_pHyprRenderer->m_renderData.pMonitor.lock())
            return {};
        auto& state = g_monitors[mon->m_id];

        bench::snapshotBegin();
        g_pHyprRenderer->pushMonitorTransformEnabled(true);
        const CBox monbox = {0, 0, mon->m_transformedSize.x, mon->m_transformedSize.y};
        const bool saved  = Render::GL::g_pHyprOpenGL->saveBufferForMirror(monbox);
        g_pHyprRenderer->popMonitorTransformEnabled();
        bench::snapshotEnd();

        if (!saved) {
            mon->resources()->invalidateMirrorFB();
            state.clean = false;
        } else if (state.clean || fullFrame(mon)) {
            mon->resources()->markMirrorFBUpdated();
            state.clean = true;
        } else {
            // Stale pixels of a hidden layer may remain outside this frame's damage:
            // ask for a full refresh and keep the black boxes until it comes.
            mon->resources()->markMirrorFBStale();
        }

#ifdef HOPP_SHARE_CLEAN_BENCH
        if (FILE* f = std::fopen("/tmp/hopp-share-trace.log", "a")) {
            std::fprintf(f, "snapshot saved=%d full=%d clean=%d held=%zu\n", saved, fullFrame(mon), state.clean, state.held.size());
            std::fclose(f);
        }
#endif
        if (state.clean) {
            for (const auto& ref : state.held) {
                if (const auto ls = ref.lock(); ls && ls->m_ruleApplicator)
                    ls->m_ruleApplicator->noScreenShare().set(false, OVERRIDE);
            }
        }
        return {};
    }

    bool needsLiveBlur() override {
        return false;
    }
    bool needsPrecomputeBlur() override {
        return false;
    }
    const char* passName() override {
        return "CSnapshotElement";
    }
    ePassElementType type() override {
        return EK_CUSTOM;
    }
    bool undiscardable() override {
        return true;
    }
    // Only when a held layer is opaque: switching simplification off draws everything
    // hidden behind windows too, every frame, which costs real GPU time at 5K.
    bool disableSimplification() override {
        return m_occlude;
    }

  private:
    PHLMONITORREF m_mon;
    bool          m_occlude = false;
};

// Tells Hyprland's end-of-frame code the mirror is already done (its copy is skipped
// for "fake" frames, and the flag is reset when the next frame begins).
class CMirrorDoneElement : public IPassElement {
  public:
    std::vector<UP<IPassElement>> draw() override {
        Render::GL::g_pHyprOpenGL->m_fakeFrame = true;
        return {};
    }
    bool needsLiveBlur() override {
        return false;
    }
    bool needsPrecomputeBlur() override {
        return false;
    }
    const char* passName() override {
        return "CMirrorDoneElement";
    }
    ePassElementType type() override {
        return EK_CUSTOM;
    }
    bool undiscardable() override {
        return true;
    }
};

static bool canHold(const PHLMONITOR& mon) {
    return mon->needsACopyFB() && !mon->needsUnmodifiedCopy() && !mon->isMirror() && !mon->m_solitaryClient.lock() &&
        g_pHyprRenderer->m_renderData.mouseZoomFactor == 1.F && !(g_pSessionLockManager && g_pSessionLockManager->isSessionLocked());
}

static void putBack(SMonitorState& state, const PHLMONITOR& mon) {
    if (!state.active)
        return;
    mon->m_layerSurfaceLayers[OVERLAY] = state.original;
    state.active                       = false;
}

static void onBegin(const PHLMONITOR& mon) {
    [[maybe_unused]] bench::SCpuTimer timer;
    auto&                             state = g_monitors[mon->m_id];
    putBack(state, mon); // never left out past a frame
    state.held.clear();
    state.original.clear();
    state.mayOcclude = false;

    // No share is waiting on this frame: Hyprland makes no copy, so there is nothing to
    // keep clean. (An override from the last shared frame stays until the next one,
    // which is the only thing that reads it.)
    if (!mon->needsACopyFB())
        return;

    std::vector<PHLLSREF> kept;
    for (const auto& ref : mon->m_layerSurfaceLayers[OVERLAY]) {
        const auto ls = ref.lock();
        if (ls && ls->m_ruleApplicator && hiddenByRule(ls)) {
            state.held.push_back(ref);
            state.mayOcclude = state.mayOcclude || opaque(ls);
        } else
            kept.push_back(ref);
    }

    if (bench::holdNothing())
        state.held.clear();

    if (state.held.empty()) {
        // Nothing to hide: Hyprland's own copy is clean. Pixels a closed layer left in
        // the mirror go when its unmapping damages that area.
        state.clean = true;
        return;
    }

#ifdef HOPP_SHARE_CLEAN_BENCH
    if (FILE* f = std::fopen("/tmp/hopp-share-trace.log", "a")) {
        std::fprintf(f, "begin copy=%d held=%zu canHold=%d clean=%d\n", mon->needsACopyFB(), state.held.size(), canHold(mon), state.clean);
        std::fclose(f);
    }
#endif
    if (!canHold(mon)) {
        // Black boxes this frame, and the mirror may now hold the layers.
        state.held.clear();
        state.clean = false;
        return;
    }

    state.original                     = mon->m_layerSurfaceLayers[OVERLAY];
    mon->m_layerSurfaceLayers[OVERLAY] = kept;
    state.active                       = true;
}

static void onLastMoment(const PHLMONITOR& mon) {
    [[maybe_unused]] bench::SCpuTimer timer;
    auto&                             state = g_monitors[mon->m_id];
    if (!state.active)
        return;
    putBack(state, mon);

    auto& pass = g_pHyprRenderer->m_renderPass;
    pass.add(makeUnique<CSnapshotElement>(mon, state.mayOcclude || bench::forceNoSimplify()));
    const auto now = Time::steadyNow();
    for (const auto& ref : state.held) {
        if (const auto ls = ref.lock())
            g_pHyprRenderer->renderLayer(ls, mon, now);
    }
    for (const auto& ref : state.held) {
        if (const auto ls = ref.lock())
            g_pHyprRenderer->renderLayer(ls, mon, now, true);
    }
    pass.add(makeUnique<CMirrorDoneElement>());
}

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    const std::string SERVER = __hyprland_api_get_hash();
    const std::string CLIENT = __hyprland_api_get_client_hash();
    if (SERVER != CLIENT) {
        HyprlandAPI::addNotification(PHANDLE, "[hopp-share-clean] built for another Hyprland: not loading. Rebuild it with `make plugin` in Hopp.", CHyprColor{1.0, 0.2, 0.2, 1.0},
                                     8000);
        throw std::runtime_error("[hopp-share-clean] version mismatch");
    }

    g_stageListener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
        const auto mon = g_pHyprRenderer->m_renderData.pMonitor.lock();
        if (!mon)
            return;
        if (stage == RENDER_BEGIN) {
            bench::frameBegin();
            onBegin(mon);
        } else if (stage == RENDER_LAST_MOMENT) {
            onLastMoment(mon);
#ifdef HOPP_SHARE_CLEAN_BENCH
            // Baseline: time Hyprland's own end-of-frame share copy by doing it here.
            if (bench::holdNothing() && mon->needsACopyFB() && !mon->needsUnmodifiedCopy()) {
                g_pHyprRenderer->m_renderPass.add(makeUnique<CSnapshotElement>(mon, false));
                g_pHyprRenderer->m_renderPass.add(makeUnique<CMirrorDoneElement>());
            }
#endif
            bench::frameEnd();
        }
    });

    return {"hopp-share-clean", "Leaves no_screen_share overlay layers out of screen shares instead of blacking them out", "Hopp", "1.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    g_stageListener.reset();
    for (const auto& ls : Desktop::layerState()->layers()) {
        if (ls && ls->m_ruleApplicator)
            ls->m_ruleApplicator->noScreenShare().unset(OVERRIDE);
    }
    g_monitors.clear();
}
