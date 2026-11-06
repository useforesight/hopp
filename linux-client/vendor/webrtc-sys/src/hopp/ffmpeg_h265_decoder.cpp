/*
 * Hopp patch: H.265 decoding on Linux through the system FFmpeg.
 * See ffmpeg_h265_decoder.h for why FFmpeg is loaded with dlopen.
 */

#include "ffmpeg_h265_decoder.h"

#include <dlfcn.h>

#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <string>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
}

#include "api/scoped_refptr.h"
#include "api/video/encoded_image.h"
#include "api/video/i420_buffer.h"
#include "api/video/video_frame.h"
#include "common_video/include/video_frame_buffer_pool.h"
#include "libyuv/convert.h"
#include "libyuv/planar_functions.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "rtc_base/logging.h"

namespace hopp {
namespace {

#define HOPP_STR2(x) #x
#define HOPP_STR(x) HOPP_STR2(x)

// Function pointers into the system FFmpeg. decltype keeps the signatures
// tied to the headers this file was built with, and the sonames below pin the
// libraries to the same major versions, so the struct layouts match.
struct Av {
  decltype(&avcodec_find_decoder) find_decoder;
  decltype(&avcodec_alloc_context3) alloc_context3;
  decltype(&avcodec_open2) open2;
  decltype(&avcodec_free_context) free_context;
  decltype(&avcodec_send_packet) send_packet;
  decltype(&avcodec_receive_frame) receive_frame;
  decltype(&av_packet_alloc) packet_alloc;
  decltype(&av_packet_free) packet_free;
  decltype(&av_frame_alloc) frame_alloc;
  decltype(&av_frame_free) frame_free;
  decltype(&av_frame_unref) frame_unref;
  decltype(&av_hwdevice_ctx_create) hwdevice_ctx_create;
  decltype(&av_hwframe_transfer_data) hwframe_transfer_data;
  decltype(&av_buffer_ref) buffer_ref;
  decltype(&av_buffer_unref) buffer_unref;
};

bool EnvDisabled(const char* name) {
  const char* value = std::getenv(name);
  return value && (std::strcmp(value, "0") == 0 ||
                   std::strcmp(value, "false") == 0 ||
                   std::strcmp(value, "off") == 0);
}

template <typename T>
bool Bind(void* lib, const char* name, T& out) {
  out = reinterpret_cast<T>(dlsym(lib, name));
  if (!out) {
    RTC_LOG(LS_WARNING) << "Hopp H265: " << name << " missing from FFmpeg";
  }
  return out != nullptr;
}

const Av* LoadAv() {
  static const Av* av = []() -> const Av* {
    if (EnvDisabled("HOPP_H265_DECODER")) {
      RTC_LOG(LS_INFO) << "Hopp H265: disabled by HOPP_H265_DECODER";
      return nullptr;
    }
    // RTLD_DEEPBIND: resolve FFmpeg's own symbols inside the system libraries,
    // never against the private FFmpeg linked into WebRTC.
    const int flags = RTLD_NOW | RTLD_LOCAL | RTLD_DEEPBIND;
    const char* avutil_name = "libavutil.so." HOPP_STR(LIBAVUTIL_VERSION_MAJOR);
    const char* avcodec_name =
        "libavcodec.so." HOPP_STR(LIBAVCODEC_VERSION_MAJOR);
    void* avutil = dlopen(avutil_name, flags);
    void* avcodec = avutil ? dlopen(avcodec_name, flags) : nullptr;
    if (!avutil || !avcodec) {
      RTC_LOG(LS_INFO) << "Hopp H265: system FFmpeg unavailable ("
                       << avutil_name << ", " << avcodec_name
                       << "): " << dlerror();
      return nullptr;
    }
    auto* loaded = new Av();
    bool ok = Bind(avcodec, "avcodec_find_decoder", loaded->find_decoder) &&
              Bind(avcodec, "avcodec_alloc_context3", loaded->alloc_context3) &&
              Bind(avcodec, "avcodec_open2", loaded->open2) &&
              Bind(avcodec, "avcodec_free_context", loaded->free_context) &&
              Bind(avcodec, "avcodec_send_packet", loaded->send_packet) &&
              Bind(avcodec, "avcodec_receive_frame", loaded->receive_frame) &&
              Bind(avcodec, "av_packet_alloc", loaded->packet_alloc) &&
              Bind(avcodec, "av_packet_free", loaded->packet_free) &&
              Bind(avutil, "av_frame_alloc", loaded->frame_alloc) &&
              Bind(avutil, "av_frame_free", loaded->frame_free) &&
              Bind(avutil, "av_frame_unref", loaded->frame_unref) &&
              Bind(avutil, "av_hwdevice_ctx_create",
                   loaded->hwdevice_ctx_create) &&
              Bind(avutil, "av_hwframe_transfer_data",
                   loaded->hwframe_transfer_data) &&
              Bind(avutil, "av_buffer_ref", loaded->buffer_ref) &&
              Bind(avutil, "av_buffer_unref", loaded->buffer_unref);
    if (!ok || !loaded->find_decoder(AV_CODEC_ID_HEVC)) {
      RTC_LOG(LS_WARNING) << "Hopp H265: system FFmpeg has no HEVC decoder";
      delete loaded;
      return nullptr;
    }
    // The libraries stay loaded for the life of the process.
    RTC_LOG(LS_INFO) << "Hopp H265: using " << avcodec_name;
    return loaded;
  }();
  return av;
}

AVPixelFormat PreferVaapi(AVCodecContext*, const AVPixelFormat* formats) {
  const AVPixelFormat* last = formats;
  for (const AVPixelFormat* f = formats; *f != AV_PIX_FMT_NONE; ++f) {
    if (*f == AV_PIX_FMT_VAAPI) return *f;
    last = f;
  }
  // FFmpeg lists hardware formats first and the software format last.
  return *last;
}

class FfmpegH265Decoder : public webrtc::VideoDecoder {
 public:
  explicit FfmpegH265Decoder(const Av* av) : av_(av) {}
  ~FfmpegH265Decoder() override { Release(); }

  bool Configure(const Settings& settings) override {
    Release();
    const AVCodec* codec = av_->find_decoder(AV_CODEC_ID_HEVC);
    ctx_ = codec ? av_->alloc_context3(codec) : nullptr;
    if (!ctx_) return false;
    ctx_->flags |= AV_CODEC_FLAG_LOW_DELAY;
    hardware_ = false;
    if (!EnvDisabled("HOPP_H265_HWDEC") &&
        av_->hwdevice_ctx_create(&device_, AV_HWDEVICE_TYPE_VAAPI, nullptr,
                                 nullptr, 0) >= 0) {
      ctx_->hw_device_ctx = av_->buffer_ref(device_);
      ctx_->get_format = PreferVaapi;
      ctx_->thread_count = 1;
      hardware_ = true;
    } else {
      // Slice threads add no delay; frame threads would hold frames back.
      ctx_->thread_count = std::clamp(settings.number_of_cores(), 1, 8);
      ctx_->thread_type = FF_THREAD_SLICE;
    }
    if (av_->open2(ctx_, codec, nullptr) < 0) {
      RTC_LOG(LS_ERROR) << "Hopp H265: avcodec_open2 failed";
      Release();
      return false;
    }
    packet_ = av_->packet_alloc();
    frame_ = av_->frame_alloc();
    transfer_ = av_->frame_alloc();
    if (!packet_ || !frame_ || !transfer_) {
      Release();
      return false;
    }
    RTC_LOG(LS_INFO) << "Hopp H265: decoder ready, "
                     << (hardware_ ? "VA-API requested" : "software");
    return true;
  }

  int32_t Decode(const webrtc::EncodedImage& image,
                 bool /*missing_frames*/,
                 int64_t /*render_time_ms*/) override {
    if (!ctx_ || !callback_) return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
    if (!image.data() || image.size() == 0)
      return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;

    // FFmpeg reads up to AV_INPUT_BUFFER_PADDING_SIZE past the end.
    input_.resize(image.size() + AV_INPUT_BUFFER_PADDING_SIZE);
    std::memcpy(input_.data(), image.data(), image.size());
    std::memset(input_.data() + image.size(), 0, AV_INPUT_BUFFER_PADDING_SIZE);
    packet_->data = input_.data();
    packet_->size = static_cast<int>(image.size());
    packet_->pts = image.RtpTimestamp();
    int result = av_->send_packet(ctx_, packet_);
    packet_->data = nullptr;
    packet_->size = 0;
    if (result < 0) {
      RTC_LOG(LS_WARNING) << "Hopp H265: send_packet failed " << result;
      return WEBRTC_VIDEO_CODEC_ERROR;
    }

    for (;;) {
      result = av_->receive_frame(ctx_, frame_);
      if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) break;
      if (result < 0) {
        RTC_LOG(LS_WARNING) << "Hopp H265: receive_frame failed " << result;
        return WEBRTC_VIDEO_CODEC_ERROR;
      }
      int32_t delivered = Deliver(frame_);
      av_->frame_unref(frame_);
      if (delivered != WEBRTC_VIDEO_CODEC_OK) return delivered;
    }
    return WEBRTC_VIDEO_CODEC_OK;
  }

  int32_t RegisterDecodeCompleteCallback(
      webrtc::DecodedImageCallback* callback) override {
    callback_ = callback;
    return WEBRTC_VIDEO_CODEC_OK;
  }

  int32_t Release() override {
    if (transfer_) av_->frame_free(&transfer_);
    if (frame_) av_->frame_free(&frame_);
    if (packet_) av_->packet_free(&packet_);
    if (ctx_) av_->free_context(&ctx_);
    if (device_) av_->buffer_unref(&device_);
    pool_.Release();
    logged_path_ = false;
    return WEBRTC_VIDEO_CODEC_OK;
  }

  DecoderInfo GetDecoderInfo() const override {
    DecoderInfo info;
    info.implementation_name =
        hardware_ ? "Hopp FFmpeg H265 (VA-API)" : "Hopp FFmpeg H265";
    info.is_hardware_accelerated = hardware_;
    return info;
  }

  const char* ImplementationName() const override {
    return hardware_ ? "Hopp FFmpeg H265 (VA-API)" : "Hopp FFmpeg H265";
  }

 private:
  int32_t Deliver(AVFrame* decoded) {
    AVFrame* source = decoded;
    const bool on_gpu = decoded->format == AV_PIX_FMT_VAAPI;
    if (on_gpu) {
      av_->frame_unref(transfer_);
      transfer_->format = AV_PIX_FMT_NV12;
      if (av_->hwframe_transfer_data(transfer_, decoded, 0) < 0) {
        RTC_LOG(LS_WARNING) << "Hopp H265: VA-API frame download failed";
        return WEBRTC_VIDEO_CODEC_ERROR;
      }
      source = transfer_;
    }
    if (!logged_path_) {
      RTC_LOG(LS_INFO) << "Hopp H265: first frame " << decoded->width << "x"
                       << decoded->height << " decoded "
                       << (on_gpu ? "in hardware (VA-API)" : "in software");
      logged_path_ = true;
    }
    hardware_ = on_gpu;

    const int width = decoded->width;
    const int height = decoded->height;
    webrtc::scoped_refptr<webrtc::I420Buffer> buffer =
        pool_.CreateI420Buffer(width, height);
    if (!buffer) return WEBRTC_VIDEO_CODEC_MEMORY;

    int converted = -1;
    switch (source->format) {
      case AV_PIX_FMT_NV12:
        converted = libyuv::NV12ToI420(
            source->data[0], source->linesize[0], source->data[1],
            source->linesize[1], buffer->MutableDataY(), buffer->StrideY(),
            buffer->MutableDataU(), buffer->StrideU(), buffer->MutableDataV(),
            buffer->StrideV(), width, height);
        break;
      case AV_PIX_FMT_YUV420P:
      case AV_PIX_FMT_YUVJ420P:
        converted = libyuv::I420Copy(
            source->data[0], source->linesize[0], source->data[1],
            source->linesize[1], source->data[2], source->linesize[2],
            buffer->MutableDataY(), buffer->StrideY(), buffer->MutableDataU(),
            buffer->StrideU(), buffer->MutableDataV(), buffer->StrideV(),
            width, height);
        break;
      default:
        RTC_LOG(LS_WARNING) << "Hopp H265: unsupported pixel format "
                            << source->format;
        return WEBRTC_VIDEO_CODEC_ERROR;
    }
    if (converted != 0) return WEBRTC_VIDEO_CODEC_ERROR;

    webrtc::VideoFrame frame =
        webrtc::VideoFrame::Builder()
            .set_video_frame_buffer(buffer)
            .set_rtp_timestamp(static_cast<uint32_t>(decoded->pts))
            .build();
    callback_->Decoded(frame, std::nullopt, std::nullopt);
    return WEBRTC_VIDEO_CODEC_OK;
  }

  const Av* av_;
  AVCodecContext* ctx_ = nullptr;
  AVBufferRef* device_ = nullptr;
  AVPacket* packet_ = nullptr;
  AVFrame* frame_ = nullptr;
  AVFrame* transfer_ = nullptr;
  std::vector<uint8_t> input_;
  webrtc::VideoFrameBufferPool pool_{false, 8};
  webrtc::DecodedImageCallback* callback_ = nullptr;
  bool hardware_ = false;
  bool logged_path_ = false;
};

}  // namespace

bool FfmpegH265DecoderAvailable() {
  return LoadAv() != nullptr;
}

std::vector<webrtc::SdpVideoFormat> FfmpegH265Formats() {
  if (!FfmpegH265DecoderAvailable()) return {};
  return {webrtc::SdpVideoFormat("H265")};
}

std::unique_ptr<webrtc::VideoDecoder> CreateFfmpegH265Decoder() {
  const Av* av = LoadAv();
  if (!av) return nullptr;
  return std::make_unique<FfmpegH265Decoder>(av);
}

}  // namespace hopp
