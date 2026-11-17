/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

use crate::{call_vpx, Error, Result};

/// Encapsulates the vp8e_enc_control_id values with their associated argument
/// types to provide enhanced type safety. Enum values prefixed with the
/// respective codec type (`Vp8` or `Vp9`) are codec-specific. Values without
/// the prefix apply to all codecs.
///
/// <div class="warning">Attempting to set a control on the wrong codec type
/// will result in an error.</div>
#[derive(Clone, Debug)]
pub enum EncoderControlSet<'a> {
    /**
     * Codec control function to pass an ROI map to encoder.
     *
     * Supported in codecs: VP8                                    
     */
    Vp8RoiMap(&'a vpx_sys::vpx_roi_map),
    /**
     * Codec control function to pass an Active map to encoder.
     *
     * Supported in codecs: VP8, VP9
     */
    ActiveMap(&'a vpx_sys::vpx_active_map),
    /**
     * Codec control function to set encoder scaling mode.
     *
     * Supported in codecs: VP8, VP9
     */
    ScaleMode(vpx_sys::vpx_scaling_mode),
    /**
     * Codec control function to set encoder internal speed settings.
     *
     * Changes in this value influences, among others, the encoder's selection
     * of motion estimation methods. Values greater than 0 will increase
     * encoder speed at the expense of quality.
     *
     * - Note: Valid range for VP8: -16..16
     * - Note: Valid range for VP9: -9..9
     * - Note: A negative value (-n) is treated as its absolute value (n) in
     *   VP9.
     *
     * Supported in codecs: VP8, VP9
     */
    CpuUsed(i32),
    /**
     * Codec control function to enable automatic use of arf frames.
     *
     * - Note: Valid range for VP8: 0..1
     * - Note: Valid range for VP9: 0..6
     *
     * Supported in codecs: VP8, VP9
     */
    EnableAutoAltRef(u32),
    /**
     * control function to set noise sensitivity
     *
     * Supported in codecs: VP8
     */
    Vp8NoiseSensitivity(Vp8NoiseSensitivity),
    /**
     * Codec control function to set higher sharpness at the expense
     * of a lower PSNR.
     *
     * - Note: Valid range: 0..7
     *
     * Supported in codecs: VP8, VP9
     */
    Sharpness(u32),
    /**
     * Codec control function to set the threshold for MBs treated static.
     *
     * Supported in codecs: VP8, VP9
     */
    StaticThreshold(u32),
    /**
     * Codec control function to set the number of token partitions.
     *
     * Supported in codecs: VP8
     */
    Vp8TokenPartitions(vpx_sys::vp8e_token_partitions),
    /**
     * Codec control function to set the max no of frames to create arf.
     *
     * Supported in codecs: VP8, VP9
     */
    ARNRMaxFrames(u32),
    /**
     * Codec control function to set the filter strength for the arf.
     *
     * Supported in codecs: VP8, VP9
     */
    ARNRStrength(u32),
    /**
     * Codec control function to set visual tuning.
     *
     * Supported in codecs: VP8, VP9
     */
    Tuning(vpx_sys::vp8e_tuning),
    /**
     * Codec control function to set constrained / constant quality level.
     *
     * <div class="warning">
     *
     * For this value to be used, you must construct
     * the encoder with [`super::EncoderConfig::rate_control`]
     * set to [`super::RateControl::ConstrainedQuality`] or
     * [`super::RateControl::ConstantQuality`].
     *
     * </div>
     *
     * Note: Valid range: 0..63
     *
     * Supported in codecs: VP8, VP9
     */
    CQLevel(u32),
    /**
     * Codec control function to set Max data rate for Intra frames.
     *
     * This value controls additional clamping on the maximum size of a
     * keyframe. It is expressed as a percentage of the average
     * per-frame bitrate, with the special (and default) value 0 meaning
     * unlimited, or no additional clamping beyond the codec's built-in
     * algorithm.
     *
     * For example, to allocate no more than 4.5 frames worth of bitrate
     * to a keyframe, set this to 450.
     *
     * Supported in codecs: VP8, VP9
     */
    MaxIntraBitratePct(u32),
    /**
     * Codec control function to set reference and update frame flags.
     *
     *  Supported in codecs: VP8
     */
    Vp8FrameFlags(i32),
    /**
     * Codec control function to set max data rate for Inter frames.
     *
     * This value controls additional clamping on the maximum size of an
     * inter frame. It is expressed as a percentage of the average
     * per-frame bitrate, with the special (and default) value 0 meaning
     * unlimited, or no additional clamping beyond the codec's built-in
     * algorithm.
     *
     * For example, to allow no more than 4.5 frames worth of bitrate
     * to an inter frame, set this to 450.
     *
     * Supported in codecs: VP9
     */
    Vp9MaxInterBitratePct(u32),
    /**
     * Boost percentage for Golden Frame in CBR mode.
     *
     * This value controls the amount of boost given to Golden Frame in
     * CBR mode. It is expressed as a percentage of the average
     * per-frame bitrate, with the special (and default) value 0 meaning
     * the feature is off, i.e., no golden frame boost in CBR mode and
     * average bitrate target is used.
     *
     * For example, to allow 100% more bits, i.e., 2X, in a golden frame
     * than average frame, set this to 100.
     *
     * Supported in codecs: VP9
     */
    Vp9GoldenFrameCBRBoostPct(u32),
    /**
     * Codec control function to set the temporal layer id.
     *
     * For temporal scalability: this control allows the application to set
     * the layer id for each frame to be encoded. Note that this control must
     * be set for every frame prior to encoding. The usage of this control
     * function supersedes the internal temporal pattern counter, which is
     * now deprecated.
     *
     * Supported in codecs: VP8
     */
    Vp8TemporalLayerID(i32),
    /**
     * Codec control function to set encoder screen content mode.
     *
     * Supported in codecs: VP8
     */
    Vp8ScreenContentMode(Vp8ScreenContentMode),
    /**
     * Codec control function to set encoding mode.
     *
     * VP9 can operate in lossless encoding mode, in which the bitstream
     * produced will be able to decode and reconstruct a perfect copy of
     * input source. This control function provides a mean to switch encoder
     * into lossless coding mode or normal coding mode that may be lossy.
     *
     * By default, encoder operates in normal coding mode (lossy).
     *
     * Supported in codecs: VP9
     */
    Vp9CodingMode(Vp9CodingMode),
    /**
     * Codec control function to set number of tile columns.
     *
     * In encoding and decoding, VP9 allows an input image frame be partitioned
     * into separated vertical tile columns, which can be encoded or decoded
     * independently. This enables easy implementation of parallel encoding and
     * decoding. This control requests the encoder to use column tiles in
     * encoding an input frame, with number of tile columns (in Log2 unit) as
     * the parameter:
     *
     * - 0 = 1 tile column
     * - 1 = 2 tile columns
     * - 2 = 4 tile columns
     * - .....
     * - n = 2<sup>n</sup> tile columns
     *
     * The requested tile columns will be capped by the encoder based on image
     * size limitations (The minimum width of a tile column is 256 pixels, the
     * maximum is 4096).
     *
     * By default, the value is 6, i.e., the maximum number of tiles supported
     * by the resolution.
     *
     * Supported in codecs: VP9
     */
    Vp9TileColumns(u32),
    /**
     * Codec control function to set number of tile rows.
     *
     * In encoding and decoding, VP9 allows an input image frame be partitioned
     * into separated horizontal tile rows. Tile rows are encoded or decoded
     * sequentially. Even though encoding/decoding of later tile rows depends
     * on earlier ones, this allows the encoder to output data packets for tile
     * rows prior to completely processing all tile rows in a frame, thereby
     * reducing the latency in processing between input and output. The
     * parameter for this control describes the number of tile rows, which has
     * a valid range [0, 2]:
     *
     * - 0 = 1 tile row
     * - 1 = 2 tile rows
     * - 2 = 4 tile rows
     *
     * By default, the value is 0, i.e. one single row tile for entire image.
     *
     * Supported in codecs: VP9
     */
    Vp9TileRows(u32),
    /**
     * Codec control function to enable frame parallel decoding feature.
     *
     * VP9 has a bitstream feature to reduce decoding dependency between frames
     * by turning off backward update of probability context used in encoding
     * and decoding. This allows staged parallel processing of more than one
     * video frame in the decoder. This control function provides a means to
     * turn this feature on or off for bitstreams produced by encoder.
     *
     * By default, this feature is on.
     *
     * Supported in codecs: VP9
     */
    Vp9FrameParallelDecoding(bool),
    /**
     * Codec control function to set adaptive quantization mode.
     *
     * VP9 has a segment based feature that allows encoder to adaptively change
     * quantization parameter for each segment within a frame to improve the
     * subjective quality. This control makes encoder operate in one of the
     * several AQ_modes supported.
     *
     * By default, encoder operates with AQ_Mode 0(adaptive quantization off).
     *
     * Supported in codecs: VP9
     */
    Vp9AQMode(Vp9AQMode),
    /**
     * Codec control function to enable/disable periodic Q boost.
     *
     * One VP9 encoder speed feature is to enable quality boost by lowering
     * frame level Q periodically. This control function provides a mean to
     * turn on/off this feature.
     *
     * By default, the encoder is allowed to use this feature for appropriate
     * encoding modes.
     *
     * Supported in codecs: VP9
     */
    Vp9FramePeriodicBoost(bool),
    /**
     * Codec control function to set noise sensitivity.
     *
     * Supported in codecs: VP9
     */
    Vp9NoiseSensitivity(Vp9NoiseSensitivity),
    /**
     * Codec control function to turn on/off SVC in encoder.
     *
     * Note: Returns an error if the encoder does not support SVC in its
     * current encoding mode
     *
     * Supported in codecs: VP9
     */
    Vp9Svc(bool),
    /**
     * Codec control function to pass an ROI map to encoder.
     *
     * Supported in codecs: VP9
     */
    Vp9RoiMap(Option<&'a vpx_sys::vpx_roi_map>),
    // VP9E_SET_SVC_PARAMETERS and VP9E_SET_SVC_LAYER_ID intentionally skipped
    // for lack of clarity of the parameter data types and how to make them
    // at least reasonably work in Rust.
    /**
     * Codec control function to set content type.
     *
     * Supported in codecs: VP9
     */
    Vp9TuneContent(Vp9TuneContent),
    /**
     * Codec control function to set color space info.
     *
     * Supported in codecs: VP9
     */
    Vp9ColorSpace(vpx_sys::vpx_color_space),
    /**
     * Codec control function to set minimum interval between GF/ARF frames.
     *
     * By default the value is set as 4.
     *
     * Supported in codecs: VP9
     */
    Vp9MinGFInterval(u32),
    /**
     * Codec control function to set miximum interval between GF/ARF frames.
     *
     * By default the value is set as 16.
     *
     * Supported in codecs: VP9
     */
    Vp9MaxGFInterval(u32),
    /**
     * Codec control function to set color range bit.
     * Supported in codecs: VP9
     */
    Vp9ColorRange(Vp9ColorRange),
    /**
     * Codec control function to set the frame flags and buffer indices
     * for spatial layers. The frame flags and buffer indices are set using the
     * struct #vpx_svc_ref_frame_config defined below.
     *
     * Supported in codecs: VP9
     */
    Vp9SvcRefFrameConfig(&'a vpx_sys::vpx_svc_ref_frame_config),
    /**
     * Codec control function to set intended rendering image size
     *
     * By default, this is identical to the image size in pixels.
     *
     * Supported in codecs: VP9
     */
    Vp9RenderSize {
        /// Rendering image width in pixels.
        width: i32,
        /// Rendering image height in pixels.
        height: i32,
    },
    /**
     * Codec control function to set target level.
     *
     * - 255: off (default)
     * - 0: only keep level stats
     * - 10: target for level 1.0
     * - 11: target for level 1.1
     * - ...
     * - 62: target for level 6.2
     *
     * Supported in codecs: VP9
     */
    Vp9TargetLevel(u8),
    /**
     * Codec control function to set row level multi-threading.
     *
     * Supported in codecs: VP9
     */
    Vp9RowMT(bool),
    /**
     * Codec control function to enable/disable special mode for altref
     * adaptive quantization. You can use it with --aq-mode concurrently.
     *
     * Enable special adaptive quantization for altref frames based on their
     * expected prediction quality for the future frames.
     *
     * Supported in codecs: VP9
     */
    Vp9AltRefAQ(bool),
    /**
     * Boost percentage for Golden Frame in CBR mode.
     *
     * This value controls the amount of boost given to Golden Frame in
     * CBR mode. It is expressed as a percentage of the average
     * per-frame bitrate, with the special (and default) value 0 meaning
     * the feature is off, i.e., no golden frame boost in CBR mode and
     * average bitrate target is used.
     *
     * For example, to allow 100% more bits, i.e., 2X, in a golden frame
     * than average frame, set this to 100.
     *
     * Supported in codecs: VP8
     */
    Vp8GoldenFrameCBRBoostPct(u32),
    // VP9E_ENABLE_MOTION_VECTOR_UNIT_TEST intentionally skipped
    /**
     * Codec control function to constrain the inter-layer prediction
     * (prediction of lower spatial resolution) in VP9 SVC.
     *
     * Supported in codecs: VP9
     */
    Vp9SvcInterLayerPred(Vp9SvcInterLayerPred),
    /**
     * Codec control function to set mode and thresholds for frame
     * dropping in SVC. Drop frame thresholds are set per-layer. Mode is set
     * as:
     * 0 : layer-dependent dropping, 1 : constrained dropping, current layer
     *  drop forces drop on all upper layers. Default mode is 0.
     *
     * Supported in codecs: VP9
     */
    Vp9SvcFrameDropLayer(vpx_sys::vpx_svc_frame_drop),
    /**
     * Codec control function to enable/disable use of golden
     * reference as a second temporal reference for SVC. Only used when
     * inter-layer prediction is disabled on INTER frames.
     *
     * Supported in codecs: VP9
     */
    Vp9SvcGFTemporalRef(bool),
    /**
     * Codec control function to enable spatial layer sync frame, for
     * any spatial layer. Enabling it for layer k means spatial layer k will
     * disable all temporal prediction, but keep the inter-layer prediction.
     * It will refresh any temporal reference buffer for that layer, and reset
     * the* temporal layer for the superframe to 0. Setting the layer sync for
     * base spatial layer forces a key frame. Default is off (0) for all spatial
     * layers. Spatial layer sync flag is reset to 0 after each encoded layer,
     * so when control is invoked it is only used for the current superframe.
     *
     * Supported in codecs: VP9
     */
    Vp9SvcSpatialLayerSync(vpx_sys::vpx_svc_spatial_layer_sync),
    /**
     * Codec control function to enable temporal dependency model.
     *
     * Vp9 allows the encoder to run temporal dependency model and use it to
     * improve the compression performance. The default value is true.
     */
    Vp9SetTPL(bool),
    /**
     * Codec control function to enable postencode frame drop.
     *
     * This will allow encoder to drop frame after it's encoded.
     * Default: false.
     *
     * Supported in codecs: VP9
     */
    Vp9PostEncodeDrop(bool),
    /**
     * Codec control function to set delta q for uv.
     *
     * Cap it at +/-15.
     *
     * Supported in codecs: VP9
     */
    Vp9DeltaQUV(i32),
    /**
     * Codec control function to disable increase Q on overshoot in CBR.
     * Default is true (Q increase is NOT allowed to overshoot CBR).
     *
     * Supported in codecs: VP9
     */
    Vp9DisableOvershootMaxQCBR(bool),
    /**
     * Codec control function to disable loopfilter.
     *
     * 0: Loopfilter on all frames, 1: Disable on non reference frames.
     * 2: Disable on all frames.
     *
     * Supported in codecs: VP9
     */
    Vp9Loopfilter(Vp9Loopfilter),
    /**
     * Specifies custom callbacks for external rate control.
     *
     * Supported in codecs: VP9
     */
    Vp9ExternalRateControl(&'a vpx_sys::vpx_rc_funcs),
    /**
     * Codec control to set quantizer for the next frame.
     *
     * This will turn off cyclic refresh. Only applicable to 1-pass without
     * spatial layers.
     *
     * Supported in codecs: VP9
     *
     */
    Vp9SetQuantizerOnePass(i32),
}

impl EncoderControlSet<'_> {
    pub(crate) fn set(&self, ctx: &mut vpx_sys::vpx_codec_ctx_t) -> Result<()> {
        macro_rules! ctrl {
            ($ctx: expr, $ctrl_id: expr$(,)?) => {
                call_vpx!(
                    vpx_sys::vpx_codec_control_($ctx, $ctrl_id as i32),
                    Error::CodecControlFailed,
                )
            };
            ($ctx: expr, $ctrl_id: expr, $arg: expr$(,)?) => {
                call_vpx!(
                    vpx_sys::vpx_codec_control_($ctx, $ctrl_id as i32, $arg),
                    Error::CodecControlFailed,
                )
            };
        }
        use vpx_sys::vp8e_enc_control_id::*;
        match self {
            Self::Vp8RoiMap(map) => ctrl!(ctx, VP8E_SET_ROI_MAP, *map),
            Self::ActiveMap(map) => ctrl!(ctx, VP8E_SET_ACTIVEMAP, *map),
            Self::ScaleMode(mode) => ctrl!(ctx, VP8E_SET_SCALEMODE, mode),
            // vesper patch: pass the value, not a reference to it, through the C varargs.
            Self::CpuUsed(cpu_used) => ctrl!(ctx, VP8E_SET_CPUUSED, *cpu_used),
            Self::EnableAutoAltRef(arf) => {
                ctrl!(ctx, VP8E_SET_ENABLEAUTOALTREF, *arf)
            }
            Self::Vp8NoiseSensitivity(sens) => {
                ctrl!(ctx, VP8E_SET_NOISE_SENSITIVITY, *sens)
            }
            Self::Sharpness(sharpness) => {
                ctrl!(ctx, VP8E_SET_SHARPNESS, *sharpness)
            }
            Self::StaticThreshold(thresh) => {
                ctrl!(ctx, VP8E_SET_STATIC_THRESHOLD, *thresh)
            }
            Self::Vp8TokenPartitions(part) => {
                ctrl!(ctx, VP8E_SET_TOKEN_PARTITIONS, *part)
            }
            Self::ARNRMaxFrames(frames) => {
                ctrl!(ctx, VP8E_SET_ARNR_MAXFRAMES, *frames)
            }
            Self::ARNRStrength(strength) => {
                ctrl!(ctx, VP8E_SET_ARNR_STRENGTH, *strength)
            }
            Self::Tuning(tuning) => ctrl!(ctx, VP8E_SET_TUNING, *tuning),
            Self::CQLevel(level) => ctrl!(ctx, VP8E_SET_CQ_LEVEL, *level),
            Self::MaxIntraBitratePct(pct) => {
                ctrl!(ctx, VP8E_SET_MAX_INTRA_BITRATE_PCT, *pct)
            }
            Self::Vp8FrameFlags(flags) => {
                ctrl!(ctx, VP8E_SET_FRAME_FLAGS, *flags)
            }
            Self::Vp9MaxInterBitratePct(pct) => {
                ctrl!(ctx, VP9E_SET_MAX_INTER_BITRATE_PCT, *pct)
            }
            Self::Vp9GoldenFrameCBRBoostPct(layer_id) => {
                ctrl!(ctx, VP9E_SET_GF_CBR_BOOST_PCT, *layer_id)
            }
            Self::Vp8TemporalLayerID(pct) => {
                ctrl!(ctx, VP8E_SET_TEMPORAL_LAYER_ID, *pct)
            }
            Self::Vp8ScreenContentMode(mode) => {
                ctrl!(ctx, VP8E_SET_SCREEN_CONTENT_MODE, *mode)
            }
            Self::Vp9CodingMode(mode) => {
                ctrl!(ctx, VP9E_SET_LOSSLESS, *mode as u32)
            }
            Self::Vp9TileColumns(cols) => {
                ctrl!(ctx, VP9E_SET_TILE_COLUMNS, *cols)
            }
            Self::Vp9TileRows(cols) => ctrl!(ctx, VP9E_SET_TILE_ROWS, *cols),
            Self::Vp9FrameParallelDecoding(flag) => {
                ctrl!(ctx, VP9E_SET_FRAME_PARALLEL_DECODING, *flag as u32)
            }
            Self::Vp9AQMode(mode) => ctrl!(ctx, VP9E_SET_AQ_MODE, *mode),
            Self::Vp9FramePeriodicBoost(flag) => {
                ctrl!(ctx, VP9E_SET_FRAME_PERIODIC_BOOST, *flag as u32)
            }
            Self::Vp9NoiseSensitivity(sens) => {
                ctrl!(ctx, VP9E_SET_NOISE_SENSITIVITY, *sens)
            }
            Self::Vp9Svc(flag) => ctrl!(ctx, VP9E_SET_SVC, *flag as i32),
            Self::Vp9RoiMap(map) => {
                ctrl!(
                    ctx,
                    VP9E_SET_ROI_MAP,
                    map.map_or(std::ptr::null(), |map| map as *const _)
                )
            }
            Self::Vp9TuneContent(content) => {
                ctrl!(ctx, VP9E_SET_TUNE_CONTENT, *content)
            }
            Self::Vp9ColorSpace(space) => {
                ctrl!(ctx, VP9E_SET_COLOR_SPACE, *space as i32)
            }
            Self::Vp9MinGFInterval(intval) => {
                ctrl!(ctx, VP9E_SET_MIN_GF_INTERVAL, *intval)
            }
            Self::Vp9MaxGFInterval(intval) => {
                ctrl!(ctx, VP9E_SET_MAX_GF_INTERVAL, *intval)
            }
            Self::Vp9ColorRange(range) => {
                ctrl!(ctx, VP9E_SET_COLOR_RANGE, *range)
            }
            Self::Vp9SvcRefFrameConfig(config) => {
                ctrl!(ctx, VP9E_SET_SVC_REF_FRAME_CONFIG, *config)
            }
            Self::Vp9RenderSize { width, height } => {
                ctrl!(ctx, VP9E_SET_RENDER_SIZE, &[width, height])
            }
            Self::Vp9TargetLevel(level) => {
                ctrl!(ctx, VP9E_SET_TARGET_LEVEL, *level as i32)
            }
            Self::Vp9RowMT(flag) => ctrl!(ctx, VP9E_SET_ROW_MT, *flag as u32),
            Self::Vp9AltRefAQ(aq) => {
                ctrl!(ctx, VP9E_SET_ALT_REF_AQ, *aq as i32)
            }
            Self::Vp8GoldenFrameCBRBoostPct(pct) => {
                ctrl!(ctx, VP8E_SET_GF_CBR_BOOST_PCT, *pct)
            }
            Self::Vp9SvcInterLayerPred(pred) => {
                ctrl!(ctx, VP9E_SET_SVC_INTER_LAYER_PRED, *pred)
            }
            Self::Vp9SvcFrameDropLayer(drop) => {
                ctrl!(ctx, VP9E_SET_SVC_FRAME_DROP_LAYER, drop)
            }
            Self::Vp9SvcGFTemporalRef(flag) => {
                ctrl!(ctx, VP9E_SET_SVC_GF_TEMPORAL_REF, *flag as u32)
            }
            Self::Vp9SvcSpatialLayerSync(sync) => {
                ctrl!(ctx, VP9E_SET_SVC_SPATIAL_LAYER_SYNC, sync)
            }
            Self::Vp9SetTPL(flag) => {
                ctrl!(ctx, VP9E_SET_TPL, *flag as i32)
            }
            Self::Vp9PostEncodeDrop(flag) => {
                ctrl!(ctx, VP9E_SET_POSTENCODE_DROP, *flag as i32)
            }
            Self::Vp9DeltaQUV(delta) => {
                ctrl!(ctx, VP9E_SET_DELTA_Q_UV, *delta)
            }
            Self::Vp9DisableOvershootMaxQCBR(flag) => {
                // CAVEAT: libvpx says 0=On, 1=Disable, so invert the bool
                // result before passing it to libvpx
                ctrl!(ctx, VP9E_SET_DELTA_Q_UV, !*flag as i32)
            }
            Self::Vp9Loopfilter(filter) => {
                ctrl!(ctx, VP9E_SET_DISABLE_LOOPFILTER, *filter as i32)
            }
            Self::Vp9ExternalRateControl(rc_funcs) => {
                ctrl!(ctx, VP9E_SET_EXTERNAL_RATE_CONTROL, *rc_funcs)
            }
            Self::Vp9SetQuantizerOnePass(qp) => {
                ctrl!(ctx, VP9E_SET_QUANTIZER_ONE_PASS, *qp)
            }
        }
    }
}

/// Control function to set noise sensitivity.
#[allow(missing_docs)]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp8NoiseSensitivity {
    Off = 0,
    OnYOnly = 1,
    OnYuv = 2,
    OnYuvAggressive = 3,
    Adaptive = 4,
}

/// VP9 noise sensitivity setting
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9NoiseSensitivity {
    /// Off
    Off = 0,
    /// On (YOnly)
    OnYOnly = 1,
    /// For SVC only, on top two spatial layers (YOnly)
    SvcOnly = 2,
}

/// Codec control function to set encoder screen content mode.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp8ScreenContentMode {
    /// Off
    Off = 0,
    /// On
    On = 1,
    /// On with more aggressive rate control.
    OnWithAggressiveRC = 2,
}

#[allow(missing_docs)]
/// Codec adaptive quantization mode
#[derive(Copy, Clone, Default, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9AQMode {
    #[default]
    No = vpx_sys::AQ_MODE::NO_AQ as u32,
    Variance = vpx_sys::AQ_MODE::VARIANCE_AQ as u32,
    Complexity = vpx_sys::AQ_MODE::COMPLEXITY_AQ as u32,
    CyclicRefresh = vpx_sys::AQ_MODE::CYCLIC_REFRESH_AQ as u32,
    Equator360 = vpx_sys::AQ_MODE::EQUATOR360_AQ as u32,
    Lookahead = vpx_sys::AQ_MODE::LOOKAHEAD_AQ as u32,
}

/// Sets the encoded content type to help optimize encoder settings
#[derive(Copy, Clone, Default, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9TuneContent {
    #[default]
    /// Regular video content (Default)
    Default = vpx_sys::vp9e_tune_content::VP9E_CONTENT_DEFAULT as u32,
    /// Screen capture content
    Screen = vpx_sys::vp9e_tune_content::VP9E_CONTENT_SCREEN as u32,
    /// Film content: improves grain retention
    Film = vpx_sys::vp9e_tune_content::VP9E_CONTENT_FILM as u32,
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
#[allow(non_camel_case_types)]
/// Sets the VP9 set color range bit.
pub enum Vp9ColorRange {
    /// Limited range (16..235 or HBD equivalent)
    Limited = 0,
    /// Full range (0..255 or HBD equivalent)
    Full = 1,
}

#[allow(missing_docs)]
/// Codec control function to constrain the inter-layer prediction
/// (prediction of lower spatial resolution) in VP9 SVC.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9SvcInterLayerPred {
    On = 0,
    Off = 1,
    OffOnlyOnNonKeyFrames = 2,
}

/// Codec control function to disable loopfilter.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9Loopfilter {
    /// Loopfilter on all frames, 1:
    EnableAll = 0,
    /// Disable on non reference frames.
    DisableNonRef = 1,
    /// Disable on all frames.
    DisableAll = 2,
}

/// Defines the VP9 encoder coding mode (lossy or lossless).
#[derive(Copy, Clone, Default, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Vp9CodingMode {
    #[default]
    /// The encoder operates in lossy (default) mode.
    Lossy = 0,
    /// The encoder operates in lossless mode.
    Lossless = 1,
}
