/*
 * Hopp patch: H.265 decoding on Linux through the system FFmpeg.
 *
 * WebRTC's prebuilt carries H.265 packetisation but no H.265 decoder outside
 * NVIDIA hardware. This decoder loads the system's libavcodec at run time and
 * decodes through VA-API when a driver offers HEVC (on Apple Silicon, the AVD
 * via libva-v4l2_request), falling back to FFmpeg's software decoder.
 *
 * libavcodec is opened with dlopen and RTLD_DEEPBIND because the WebRTC
 * archive carries its own private FFmpeg (H.264 only) with the same symbol
 * names. Nothing here links against FFmpeg.
 */

#pragma once

#include <memory>
#include <vector>

#include "api/video_codecs/sdp_video_format.h"
#include "api/video_codecs/video_decoder.h"

namespace hopp {

// True once the system libavcodec loads and has an HEVC decoder.
// HOPP_H265_DECODER=0 turns the decoder off.
bool FfmpegH265DecoderAvailable();

// The SDP formats to advertise when the decoder is available.
std::vector<webrtc::SdpVideoFormat> FfmpegH265Formats();

std::unique_ptr<webrtc::VideoDecoder> CreateFfmpegH265Decoder();

}  // namespace hopp
