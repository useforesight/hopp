// Benchmark build only (make bench): GPU time of each frame's render pass, measured with
// GL timer queries, and the CPU time of this plugin's own callbacks. Writes a summary to
// $HOPP_SHARE_CLEAN_BENCH (default /tmp/hopp-share-bench.log) every 300 frames.
//
// $HOPP_SHARE_CLEAN_MODE picks what to compare:
//   off          hold nothing: Hyprland's own black boxes (the baseline)
//   nosimplify   the plugin with Hyprland's occlusion culling always off (the old build)
//   (unset)      the plugin as shipped
#pragma once

#include <GLES3/gl3.h>
#include <EGL/egl.h>
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <string>
#include <vector>

namespace bench {
    constexpr GLenum TIME_ELAPSED = 0x88BF; // GL_TIME_ELAPSED_EXT
    using PFNGetQueryObjectui64v  = void (*)(GLuint, GLenum, GLuint64*);

    inline std::string mode() {
        const char* env = std::getenv("HOPP_SHARE_CLEAN_MODE");
        return env ? env : "auto";
    }
    inline bool holdNothing() {
        static const bool value = mode() == "off";
        return value;
    }
    inline bool forceNoSimplify() {
        static const bool value = mode() == "nosimplify";
        return value;
    }

    // Segments of a frame: 0 = whole pass (or the scene before the snapshot), 1 = the
    // snapshot copy, 2 = the held layers after it.
    struct SPending {
        GLuint query;
        int    segment;
    };
    struct SState {
        std::deque<SPending>   pending;
        std::vector<double>    gpuUs;
        double                 segUs[3]   = {0, 0, 0};
        size_t                 segN[3]    = {0, 0, 0};
        int                    openSeg    = 0;
        double                 frameUs    = 0;
        int                    frameParts = 0;
        std::vector<double>    cpuUs;
        double                 cpuFrame = 0;
        PFNGetQueryObjectui64v getResult = nullptr;
        GLuint                 open      = 0;
    };
    inline SState& state() {
        static SState s;
        return s;
    }

    inline double percentile(std::vector<double> values, double p) {
        if (values.empty())
            return 0;
        std::ranges::sort(values);
        return values[std::min(values.size() - 1, (size_t)(p * values.size()))];
    }

    inline void report() {
        auto& s = state();
        if (s.gpuUs.size() < 300)
            return;
        double sum = 0;
        for (double v : s.gpuUs)
            sum += v;
        double cpu = 0;
        for (double v : s.cpuUs)
            cpu += v;
        const char* path = std::getenv("HOPP_SHARE_CLEAN_BENCH");
        if (FILE* f = std::fopen(path ? path : "/tmp/hopp-share-bench.log", "a")) {
            auto seg = [&](int i) { return s.segN[i] ? s.segUs[i] / s.segN[i] : 0.0; };
            std::fprintf(f, "mode=%s frames=%zu gpu_mean_us=%.1f gpu_p50_us=%.1f gpu_p95_us=%.1f scene_us=%.1f snapshot_us=%.1f held_us=%.1f plugin_cpu_mean_us=%.2f plugin_cpu_p95_us=%.2f\n",
                         mode().c_str(), s.gpuUs.size(), sum / s.gpuUs.size(), percentile(s.gpuUs, 0.5), percentile(s.gpuUs, 0.95), seg(0), seg(1), seg(2),
                         cpu / std::max<size_t>(1, s.cpuUs.size()), percentile(s.cpuUs, 0.95));
            for (int i = 0; i < 3; i++) {
                s.segUs[i] = 0;
                s.segN[i]  = 0;
            }
            std::fclose(f);
        }
        s.gpuUs.clear();
        s.cpuUs.clear();
    }

    inline void collect() {
        auto& s = state();
        while (!s.pending.empty()) {
            GLuint available = 0;
            const auto [query, segment] = s.pending.front();
            glGetQueryObjectuiv(query, GL_QUERY_RESULT_AVAILABLE, &available);
            if (!available)
                break;
            GLuint64 ns = 0;
            s.getResult(query, GL_QUERY_RESULT, &ns);
            glDeleteQueries(1, &query);
            s.pending.pop_front();
            if (segment < 0) { // frame marker: total of the parts since the last marker
                s.gpuUs.push_back(s.frameUs);
                s.frameUs = 0;
                continue;
            }
            s.segUs[segment] += ns / 1000.0;
            s.segN[segment]++;
            s.frameUs += ns / 1000.0;
        }
        report();
    }

    inline bool ready() {
        auto& s = state();
        if (!s.getResult)
            s.getResult = (PFNGetQueryObjectui64v)eglGetProcAddress("glGetQueryObjectui64vEXT");
        return s.getResult;
    }
    inline void openSegment(int segment) {
        auto& s = state();
        if (!ready() || s.open)
            return;
        glGenQueries(1, &s.open);
        glBeginQuery(TIME_ELAPSED, s.open);
        s.openSeg = segment;
    }
    inline void closeSegment() {
        auto& s = state();
        if (!s.open)
            return;
        glEndQuery(TIME_ELAPSED);
        s.pending.push_back({s.open, s.openSeg});
        s.open = 0;
    }
    // A zero-length marker query, so collect() knows where each frame ends.
    inline void endFrame() {
        auto& s = state();
        if (!ready())
            return;
        GLuint q = 0;
        glGenQueries(1, &q);
        glBeginQuery(TIME_ELAPSED, q);
        glEndQuery(TIME_ELAPSED);
        s.pending.push_back({q, -1});
    }
    inline void snapshotBegin() {
        closeSegment();
        openSegment(1);
    }
    inline void snapshotEnd() {
        closeSegment();
        openSegment(2);
    }

    class CTimerElement : public IPassElement {
      public:
        explicit CTimerElement(bool start) : m_start(start) {}
        std::vector<UP<IPassElement>> draw() override {
            if (m_start)
                openSegment(0);
            else {
                closeSegment();
                endFrame();
                collect();
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
            return "CTimerElement";
        }
        ePassElementType type() override {
            return EK_CUSTOM;
        }
        bool undiscardable() override {
            return true;
        }

      private:
        bool m_start;
    };

    inline void frameBegin() {
        state().cpuFrame = 0;
        g_pHyprRenderer->m_renderPass.add(makeUnique<CTimerElement>(true));
    }
    inline void frameEnd() {
        g_pHyprRenderer->m_renderPass.add(makeUnique<CTimerElement>(false));
        state().cpuUs.push_back(state().cpuFrame);
    }

    struct SCpuTimer {
        std::chrono::steady_clock::time_point start = std::chrono::steady_clock::now();
        ~SCpuTimer() {
            state().cpuFrame += std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - start).count();
        }
    };
}
