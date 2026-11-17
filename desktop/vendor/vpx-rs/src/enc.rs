/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

use bitflags::bitflags;
use std::ffi::c_void;
use std::fmt::Debug;
use std::mem::MaybeUninit;
use std::num::NonZero;
use std::time::Duration;
use vpx_sys::vpx_codec_enc_cfg;

use crate::common::StreamInfo;
use crate::image::{YUVImageData, YUVPixelType};
use crate::NoDebugWrapper;
use crate::{call_vpx, call_vpx_ptr};
use crate::{Error, Result};
use ctrl::EncoderControlSet;

/// Contains definitions for the various control options which can be
/// passed to [`Encoder::codec_control_set()`].
pub mod ctrl;

#[allow(missing_docs)]
/// Specifies the codec to be used for encoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum CodecId {
    VP8,
    VP9,
}

impl CodecId {
    pub(crate) fn iface(&self) -> Result<*const vpx_sys::vpx_codec_iface_t> {
        match self {
            Self::VP8 => call_vpx_ptr!(
                vpx_sys::vpx_codec_vp8_cx(),
                Error::CodecInterfaceNotAvailable,
            ),
            Self::VP9 => call_vpx_ptr!(
                vpx_sys::vpx_codec_vp9_cx(),
                Error::CodecInterfaceNotAvailable,
            ),
        }
    }
}

/// The main VP8/VP9 video encoder.
///
/// # Example
/// The example below expects the input image in YUV 4:2:0 planar format.
/// While the encoder supports a number of YUV format variants
/// (see [`crate::image::ImageFormat`]), if you are working non-YUV formats
/// (e.g. RGB), you will first have to convert your data to one of the YUV
/// formats supported by the encoder. The
/// [yuv](https://docs.rs/yuv/latest/yuv/#functions) crate
/// contains many highly optimized pixel format conversion functions. See
/// [`crate::ImageFormat`] for an example.
///
/// Note that the encoder below has most of its settings set to their default
/// values for the codec. The [`EncoderConfig`] struct gives you a lot more
/// more tuning options.
///
/// ```
/// use std::num::NonZero;
/// use vpx_rs::{
///     Encoder, EncoderConfig, EncoderFrameFlags, Error, Packet,
///     RateControl, Timebase,
/// };
///
/// // Initializes a VP8 encoder instance.
/// fn new_vp8_encoder(width: u32, height: u32) -> Result<Encoder<u8>, Error> {
///     // Start by preparing the encoder configuration.
///     let mut config = EncoderConfig::<u8>::new(
///         vpx_rs::enc::CodecId::VP8,
///         width,
///         height,
///         // assume a framerate of 30 fps, implying a 1/30 timebase
///         Timebase {
///             num: NonZero::new(1).unwrap(),
///             den: NonZero::new(30).unwrap(),
///         },
///         RateControl::ConstantBitRate(8000),
///     )?;
///     // Construct the actual encoder and return the result.
///     Encoder::new(config)
/// }
///
/// // Encodes a single frame in YUV 4:2:0 planar format and sends the
/// // compressed bitstream to a writer.
/// fn encode_yuv420_frame<W: std::io::Write>(
///     encoder: &mut Encoder<u8>,
///     frame_counter: i64,
///     width: u32,
///     height: u32,
///     yuv420_pixels: &[u8],
///     output_stream: &mut W,
/// ) -> Result<(), Box<dyn std::error::Error>> {
///     // Wrap the input buffer in an ImageData structure, together
///     // with information about the data format and dimensions.
///     let image = vpx_rs::YUVImageData::<u8>::from_raw_data(
///         vpx_rs::ImageFormat::I420,
///         width as usize,
///         height as usize,
///         yuv420_pixels,
///     )?;
///     // Send the image to the encoder for compression.
///     let packets = encoder.encode(
///         frame_counter,
///         1,
///         image,
///         vpx_rs::EncodingDeadline::default(),
///         EncoderFrameFlags::empty(),
///     )?;
///     // The encoder can spit out a series of various data packets.
///     for packet in packets {
///         match packet {
///             // CompressedFrame packets contain the actual compressed
///             // video bitstream, which we need to send to the output.
///             Packet::CompressedFrame(frame) => {
///                 println!(
///                     "Encoder produced frame with {} bytes, pts: {}",
///                     frame.data.len(),
///                     frame.pts,
///                 );
///                 // send the compressed video data to the output
///                 output_stream.write(&frame.data)?;
///             },
///             // Remaining packet types contain various metainformation.
///             // Most of these need to be enabled manually by calling
///             // Encoder::codec_control_set() and setting various
///             // advanced encoder options.
///             _ => {
///                 println!(
///                     "Encoder also produced something else: {packet:?}",
///                 );
///             }
///         }
///     }
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct Encoder<T: YUVPixelType> {
    ctx: NoDebugWrapper<vpx_sys::vpx_codec_ctx_t>,
    input_pixel_type: std::marker::PhantomData<T>,
}

// This is safe to implement, because we never hold any libvpx mutexes
// between function calls, and this crate is entirely sync code. Also,
// we have no interior mutability through immutable references.
unsafe impl<T: YUVPixelType> Send for Encoder<T> {}
unsafe impl<T: YUVPixelType> Sync for Encoder<T> {}

impl<T: YUVPixelType> Encoder<T> {
    /// Constructs a new encoder.
    pub fn new(config: EncoderConfig<T>) -> Result<Self> {
        let (iface, cfg) = unsafe { config.cfg()? };
        let mut ctx = unsafe {
            MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::zeroed().assume_init()
        };
        call_vpx!(
            vpx_sys::vpx_codec_enc_init_ver(
                &mut ctx,
                iface,
                &cfg,
                config.flags.bits() as _,
                vpx_sys::VPX_ENCODER_ABI_VERSION as _,
            ),
            Error::VpxCodecInitFailed,
        )?;
        let mut encoder = Self {
            ctx: NoDebugWrapper(ctx),
            input_pixel_type: std::marker::PhantomData,
        };
        match config.rate_control {
            RateControl::ConstrainedQuality {
                max_quality: qual, ..
            }
            | RateControl::ConstantQuality(qual) => {
                encoder.codec_control_set(EncoderControlSet::CQLevel(qual))?;
            }
            RateControl::Lossless => match config.codec {
                CodecId::VP8 => return Err(Error::UnsupportedRateControlMode),
                CodecId::VP9 => {
                    encoder.codec_control_set(
                        EncoderControlSet::Vp9CodingMode(
                            ctrl::Vp9CodingMode::Lossless,
                        ),
                    )?;
                }
            },
            _ => (),
        }
        Ok(encoder)
    }
    /// Encodes a frame of image data.
    ///
    /// - `pts`: The presentation timestamp of this frame. This should be
    ///   specified in the timebase units set up in the encoder config.
    /// - `duration`: Duration to show frame, in timebase units.
    /// - `image`: The image to be encoded.
    /// - `deadline`: The encoding deadline to pass to the encoder.
    ///
    /// Returns an interator of stream packets which were generated from this
    /// frame. Please note that the encoder need not generate new packets for
    /// every frame passed in. It can buffer multiple frames and then emit
    /// multiple stream packets at once. The `deadline` argument hints the
    /// encoder how much it should trade off reduced latency for better
    /// compression.
    pub fn encode(
        &mut self,
        pts: i64,
        duration: u64,
        image: YUVImageData<T>,
        deadline: EncodingDeadline,
        flags: EncoderFrameFlags,
    ) -> Result<PacketIterator> {
        let wrapped = unsafe { image.vpx_img_wrap()? };
        call_vpx!(
            vpx_sys::vpx_codec_encode(
                &mut self.ctx.0,
                &wrapped,
                pts as _,
                duration as _,
                flags.bits() as _,
                deadline.into(),
            ),
            Error::ErrorEncodingImage,
        )?;
        Ok(PacketIterator {
            ctx: &mut self.ctx.0,
            iter: std::ptr::null(),
        })
    }
    /// Returns information about the stream being produced.
    pub fn stream_info(&mut self) -> Option<StreamInfo> {
        crate::common::get_stream_info(&mut self.ctx.0)
    }
    /// Provides fine-grained codec control after initialization.
    pub fn codec_control_set(
        &mut self,
        control: ctrl::EncoderControlSet,
    ) -> Result<()> {
        control.set(&mut self.ctx.0)
    }
}

impl<T: YUVPixelType> Drop for Encoder<T> {
    fn drop(&mut self) {
        // We assert here, because the only documented ways for this to fail
        // are when passing a null pointer (which we cannot do) and the context
        // not being initialized (which by design cannot happen in Rust).
        unsafe {
            let error = vpx_sys::vpx_codec_destroy(&mut self.ctx.0);
            assert_eq!(error, vpx_sys::VPX_CODEC_OK);
        }
    }
}

/// Specifies an initial configuration of the encoder. Fields prefixed
/// with `rc_` denote rate-control parameters.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EncoderConfig<T: YUVPixelType> {
    /// The codec to utilize when producing a bitstream.
    pub codec: CodecId,

    /// Bitstream profile to use.
    pub profile: EncoderProfile,

    /// Flags to configure on the encoder.
    pub flags: EncoderFlags,

    /// The display width of the video stream in pixels.
    pub width: u32,

    /// The display height of the video stream in pixels.
    pub height: u32,

    /// The timebase for the presentation timestamps passed to the encoder.
    pub timebase: Timebase,

    /// The type of rate control to employ.
    pub rate_control: RateControl,

    /// Number of threads to employ in encoding.
    pub threads: u32,

    /// Bit-depth of the codec.
    ///
    /// This value identifies the bit_depth of the codec and the generated
    /// output bitstream. It doesn't imply a specific input data bit depth
    /// (which is defined by the associated generic type of this struct).
    /// The default is `VPX_BITS_8`.
    pub output_bit_depth: vpx_sys::vpx_bit_depth,

    /// Bitstream error resiliency flags.
    pub error_resilient: ErrorResilient,

    /// Multi-pass encoding support:
    /// - `VPX_RC_ONE_PASS`: Single-pass mode (default).
    /// - `VPX_RC_FIRST_PASS`: First pass of multi-pass mode.
    /// - `VPX_RC_LAST_PASS`: Final pass of multi-pass mode.
    pub pass: vpx_sys::vpx_enc_pass,

    /// Allow lagged encoding
    ///
    /// If set, this value allows the encoder to consume a number of input
    /// frames before producing output frames. This allows the encoder to
    /// base decisions for the current frame on future frames. This does
    /// increase the latency of the encoding pipeline, so it is not
    /// appropriate in all situations (ex: realtime encoding).
    ///
    /// Note that this is a maximum value -- the encoder may produce frames
    /// sooner than the given limit. Set this value to 0 to disable this
    /// feature (this is the default).
    pub lag_in_frames: u32,

    /// Temporal resampling configuration, if supported by the codec.
    ///
    /// Temporal resampling allows the codec to "drop" frames as a strategy to
    /// meet its target data rate. This can cause temporal discontinuities in
    /// the encoded video, which may appear as stuttering during playback.
    /// This trade-off is often acceptable, but for many applications is not.
    /// It can be disabled in these cases.
    ///
    /// This threshold is described as a percentage of the target data buffer.
    /// When the data buffer falls below this percentage of fullness, a
    /// dropped frame is indicated. Set the threshold to zero (0) to disable
    /// this feature.
    pub rc_dropframe_thresh: u32,

    /// Enable/disable spatial resampling, if supported by the codec.
    ///
    /// Spatial resampling allows the codec to compress a lower resolution
    /// version of the frame, which is then upscaled by the encoder to the
    /// correct presentation resolution. This increases visual quality at
    /// low data rates, at the expense of CPU time on the encoder/decoder.
    ///
    /// If set to `None`, spatial resizing is disallowed.
    pub rc_resize_allowed: Option<SpatialResizeParams>,

    /// Two-pass stats buffer.
    ///
    /// A buffer containing all of the stats packets produced in the first
    /// pass, concatenated.
    ///
    /// If set to `None` (the default), no two-pass stats are passed in.
    pub rc_twopass_stats_in: Option<FixedBuffer>,

    /// First pass mb stats buffer.
    ///
    /// A buffer containing all of the first pass mb stats packets produced
    /// in the first pass, concatenated.
    ///
    /// If set to `None` (the default), no first-pass MB stats are passed in.
    pub rc_firstpass_mb_stats_in: Option<FixedBuffer>,

    /// Rate control adaptation undershoot control
    ///
    /// VP8: Expressed as a percentage of the target bitrate, controls the
    /// maximum allowed adaptation speed of the codec. This factor controls
    /// the maximum amount of bits that can be subtracted from the target
    /// bitrate in order to compensate for prior overshoot.
    ///
    /// VP9: Expressed as a percentage of the target bitrate, a threshold
    /// undershoot level (current rate vs target) beyond which more aggressive
    /// corrective measures are taken.
    ///
    /// Valid values in the range VP8:0-100 VP9: 0-100.
    pub rc_undershoot_pct: u32,

    /// Rate control adaptation overshoot control
    ///
    /// VP8: Expressed as a percentage of the target bitrate, controls the
    /// maximum allowed adaptation speed of the codec. This factor controls
    /// the maximum amount of bits that can be added to the target bitrate in
    /// order to compensate for prior undershoot.
    ///
    /// VP9: Expressed as a percentage of the target bitrate, a threshold
    /// overshoot level (current rate vs target) beyond which more aggressive
    /// corrective measures are taken.
    ///
    /// Valid values in the range VP8:0-100 VP9: 0-100.
    pub rc_overshoot_pct: u32,

    /// Decoder Buffer Size
    ///
    /// This value indicates the amount of data that may be buffered by the
    /// decoding application. Note that this value is expressed in units of
    /// time.
    pub rc_buf_sz: Duration,

    /// Decoder Buffer Initial Size
    ///
    /// This value indicates the amount of data that will be buffered by the
    /// decoding application prior to beginning playback. This value is
    /// expressed in units of time
    pub rc_buf_initial_sz: Duration,

    /// Decoder Buffer Optimal Size
    ///
    /// This value indicates the amount of data that the encoder should try
    /// to maintain in the decoder's buffer. This value is expressed in units
    /// of time.
    pub rc_buf_optimal_sz: Duration,

    /// Two-pass mode CBR/VBR bias
    ///
    /// Bias, expressed on a scale of 0 to 100, for determining target size
    /// for the current frame. The value 0 indicates the optimal CBR mode
    /// value should be used. The value 100 indicates the optimal VBR mode
    /// value should be used. Values in between indicate which way the
    /// encoder should "lean."
    pub rc_2pass_vbr_bias_pct: u32,

    /// Two-pass mode per-GOP minimum bitrate
    ///
    /// This value, expressed as a percentage of the target bitrate, indicates
    /// the minimum bitrate to be used for a single GOP (aka "section")
    pub rc_2pass_vbr_minsection_pct: u32,

    /// Two-pass mode per-GOP maximum bitrate
    ///
    /// This value, expressed as a percentage of the target bitrate, indicates
    /// the maximum bitrate to be used for a single GOP (aka "section")
    pub rc_2pass_vbr_maxsection_pct: u32,

    /// Two-pass corpus vbr mode complexity control
    ///
    /// Used only in VP9: A value representing the corpus midpoint complexity
    /// for corpus vbr mode. This value defaults to 0 which disables corpus vbr
    /// mode in favour of normal vbr mode.
    pub rc_2pass_vbr_corpus_complexity: u32,

    /// Keyframe placement mode
    ///
    /// This value indicates whether the encoder should place keyframes at a
    /// fixed interval, or determine the optimal placement automatically
    /// (as governed by the `min_dist` and `max_dist` parameters).
    pub kf_mode: KeyFrameMode,

    /// Configures the spatial coding layers to be used by the encoder.
    pub spatial_layers: Vec<SpatialLayer>,

    /// Temporal coding layers.
    ///
    /// <div class="warning">
    ///
    /// This vector MUST NOT contain more than [`vpx_sys::VPX_TS_MAX_LAYERS`]
    /// temporal layers.
    ///
    /// </div>
    pub temporal_layers: Vec<TemporalLayer>,

    /// Template defining the membership of frames to temporal layers.
    ///
    /// This vector defines the membership of frames to temporal coding layers.
    /// For a 2-layer encoding that assigns even numbered frames to one
    /// temporal layer (0) and odd numbered frames to a second temporal layer
    /// (1), then `cfg.temporal_periodicity = vec![0,1,0,1,0,1,0,1]`.
    ///
    /// <div class="warning">
    ///
    /// This vector MUST NOT contain more than
    /// [`vpx_sys::VPX_TS_MAX_PERIODICITY`] items.
    ///
    /// </div>
    pub temporal_periodicity: Vec<u32>,

    /// Target bitrate for each spatial/temporal layer.
    ///
    /// These values specify the target coding bitrate to be used for each
    /// spatial/temporal layer (in kbps).
    pub layer_target_bitrate: [u32; vpx_sys::VPX_MAX_LAYERS as usize],

    /// Temporal layering mode indicating which temporal layering scheme to use.
    ///
    /// <div class="warning">This enum will only be populated when using VP9,
    /// since VP8 doesn't support defining the temporal layering mode.
    ///
    /// If this is set to `None`, the codec-default layering mode is
    /// used.</div>
    pub temporal_layering_mode: Option<vpx_sys::vp9e_temporal_layering_mode>,

    input_pixel_type: std::marker::PhantomData<T>,
}

impl<T: YUVPixelType> EncoderConfig<T> {
    /// Constructs a new EncoderConfig, with most values set to their defaults.
    ///
    /// <div class="warning">Note: all bitrates are in kbit/s.</div>
    pub fn new(
        codec: CodecId,
        width: u32,
        height: u32,
        timebase: Timebase,
        rate_control: RateControl,
    ) -> Result<Self> {
        // Generate the default configuration and read back its values
        // into our state.
        let mut cfg = unsafe { MaybeUninit::zeroed().assume_init() };
        call_vpx!(
            vpx_sys::vpx_codec_enc_config_default(codec.iface()?, &mut cfg, 0),
            Error::EncoderConfigInitFailed,
        )?;
        let temporal_layering_mode = if codec == CodecId::VP9 {
            Self::try_from_temporal_layering_mode(cfg.temporal_layering_mode)
        } else {
            None
        };
        Ok(Self {
            // Global params
            codec,
            profile: EncoderProfile::default(),
            flags: EncoderFlags::empty(),
            width,
            height,
            timebase,
            rate_control,
            threads: cfg.g_threads,
            output_bit_depth: cfg.g_bit_depth,
            error_resilient: ErrorResilient::empty(),
            pass: cfg.g_pass,
            lag_in_frames: cfg.g_lag_in_frames,

            // Rate control params
            rc_dropframe_thresh: cfg.rc_dropframe_thresh,
            rc_resize_allowed: SpatialResizeParams::from_cfg(&cfg),
            rc_twopass_stats_in: None,
            rc_firstpass_mb_stats_in: None,
            rc_undershoot_pct: cfg.rc_undershoot_pct,
            rc_overshoot_pct: cfg.rc_overshoot_pct,
            rc_buf_sz: Duration::from_millis(cfg.rc_buf_sz as u64),
            rc_buf_initial_sz: Duration::from_millis(
                cfg.rc_buf_initial_sz as u64,
            ),
            rc_buf_optimal_sz: Duration::from_millis(
                cfg.rc_buf_optimal_sz as u64,
            ),
            rc_2pass_vbr_bias_pct: cfg.rc_2pass_vbr_bias_pct,
            rc_2pass_vbr_minsection_pct: cfg.rc_2pass_vbr_minsection_pct,
            rc_2pass_vbr_maxsection_pct: cfg.rc_2pass_vbr_maxsection_pct,
            rc_2pass_vbr_corpus_complexity: cfg.rc_2pass_vbr_corpus_complexity,

            // Key frame control
            kf_mode: KeyFrameMode::from_cfg(&cfg),

            // Spatial scalability settings (ss)
            spatial_layers: SpatialLayer::from_cfg(&cfg),

            // Temporal scalability settings (ts)
            temporal_layers: TemporalLayer::from_cfg(&cfg),
            temporal_periodicity: Self::ts_periodicity_from_cfg(&cfg),

            layer_target_bitrate: cfg.layer_target_bitrate,
            temporal_layering_mode,
            // Type marker
            input_pixel_type: std::marker::PhantomData,
        })
    }
    /// # Safety
    ///
    /// If the configuration contained `rc_twopass_stats_in` or
    /// `rc_firstpass_mb_stats_in`, the generated `vpx_codec_enc_cfg` will
    /// reference that data by pointer. Care must be taken NOT to drop
    /// the `EncoderConfig` before the `Encoder` has been constructed (at
    /// which point the stats will be read).
    unsafe fn cfg(
        &self,
    ) -> Result<(
        *const vpx_sys::vpx_codec_iface_t,
        vpx_sys::vpx_codec_enc_cfg,
    )> {
        let iface = self.codec.iface()?;
        let mut cfg = unsafe { MaybeUninit::zeroed().assume_init() };
        call_vpx!(
            vpx_sys::vpx_codec_enc_config_default(iface, &mut cfg, 0),
            Error::EncoderConfigInitFailed,
        )?;

        // Global settings
        cfg.g_w = self.width;
        cfg.g_h = self.height;
        cfg.g_timebase.num = self.timebase.num.get() as _;
        cfg.g_timebase.den = self.timebase.den.get() as _;
        cfg.g_threads = self.threads;
        cfg.g_error_resilient = self.error_resilient.bits();
        cfg.g_pass = self.pass;
        cfg.g_bit_depth = self.output_bit_depth;
        cfg.g_input_bit_depth = (size_of::<T>() * 8) as u32;
        cfg.g_lag_in_frames = self.lag_in_frames;
        if self.profile != EncoderProfile::Default && self.codec == CodecId::VP8
        {
            return Err(Error::InvalidProfileSelected);
        }
        cfg.g_profile = self.profile as _;

        // Rate control settings
        match self.rate_control {
            RateControl::VariableBitRate(bitrate) => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_VBR;
                cfg.rc_target_bitrate = bitrate;
            }
            RateControl::ConstantBitRate(bitrate) => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_CBR;
                cfg.rc_target_bitrate = bitrate;
            }
            RateControl::ConstrainedQuality { bitrate, .. } => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_CQ;
                cfg.rc_target_bitrate = bitrate;
            }
            RateControl::ConstantQuality(_) => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_Q;
            }
            _ => (),
        }
        cfg.rc_dropframe_thresh = self.rc_dropframe_thresh;
        SpatialResizeParams::to_cfg(&self.rc_resize_allowed, &mut cfg);
        if let Some(stats) = &self.rc_twopass_stats_in {
            // WARNING: care must be taken NOT to drop the `EncoderConfig`
            // before the `Encoder` has been constructed!
            cfg.rc_twopass_stats_in = vpx_sys::vpx_fixed_buf {
                buf: stats.0.as_ptr() as *mut c_void,
                sz: stats.0.len(),
            };
        }
        if let Some(stats) = &self.rc_firstpass_mb_stats_in {
            // WARNING: care must be taken NOT to drop the `EncoderConfig`
            // before the `Encoder` has been constructed!
            cfg.rc_firstpass_mb_stats_in = vpx_sys::vpx_fixed_buf {
                buf: stats.0.as_ptr() as *mut c_void,
                sz: stats.0.len(),
            };
        }
        cfg.rc_undershoot_pct = self.rc_undershoot_pct;
        cfg.rc_overshoot_pct = self.rc_overshoot_pct;
        cfg.rc_buf_sz = self.rc_buf_sz.as_millis() as u32;
        cfg.rc_buf_initial_sz = self.rc_buf_initial_sz.as_millis() as u32;
        cfg.rc_buf_optimal_sz = self.rc_buf_optimal_sz.as_millis() as u32;
        cfg.rc_2pass_vbr_bias_pct = self.rc_2pass_vbr_bias_pct;
        cfg.rc_2pass_vbr_minsection_pct = self.rc_2pass_vbr_minsection_pct;
        cfg.rc_2pass_vbr_maxsection_pct = self.rc_2pass_vbr_maxsection_pct;
        cfg.rc_2pass_vbr_corpus_complexity =
            self.rc_2pass_vbr_corpus_complexity;

        // Key frame control
        self.kf_mode.to_cfg(&mut cfg);

        // Spatial scalability settings (ss)
        SpatialLayer::to_cfg(&self.spatial_layers, &mut cfg);

        // Temporal scalability settings (ss)
        TemporalLayer::to_cfg(&self.temporal_layers, &mut cfg);
        assert!(
            self.temporal_periodicity.len()
                < vpx_sys::VPX_TS_MAX_PERIODICITY as usize
        );
        let n_ts_periodicity = self.temporal_periodicity.len();
        cfg.ts_periodicity = n_ts_periodicity as u32;
        cfg.ts_layer_id[..n_ts_periodicity]
            .copy_from_slice(&self.temporal_periodicity[..n_ts_periodicity]);

        cfg.layer_target_bitrate = self.layer_target_bitrate;
        if let Some(x) = self.temporal_layering_mode {
            cfg.temporal_layering_mode = x as i32;
        }

        Ok((iface, cfg))
    }
    fn ts_periodicity_from_cfg(cfg: &vpx_sys::vpx_codec_enc_cfg) -> Vec<u32> {
        let mut periodicity = vec![];
        for i in 0..cfg.ts_periodicity as usize {
            periodicity.push(cfg.ts_layer_id[i]);
        }
        periodicity
    }
    fn try_from_temporal_layering_mode(
        value: i32,
    ) -> Option<vpx_sys::vp9e_temporal_layering_mode> {
        use vpx_sys::vp9e_temporal_layering_mode::*;
        match value {
            x if x == VP9E_TEMPORAL_LAYERING_MODE_NOLAYERING as i32 => {
                Some(VP9E_TEMPORAL_LAYERING_MODE_NOLAYERING)
            }
            x if x == VP9E_TEMPORAL_LAYERING_MODE_BYPASS as i32 => {
                Some(VP9E_TEMPORAL_LAYERING_MODE_BYPASS)
            }
            x if x == VP9E_TEMPORAL_LAYERING_MODE_0101 as i32 => {
                Some(VP9E_TEMPORAL_LAYERING_MODE_0101)
            }
            x if x == VP9E_TEMPORAL_LAYERING_MODE_0212 as i32 => {
                Some(VP9E_TEMPORAL_LAYERING_MODE_0212)
            }
            _ => None,
        }
    }
}

/// If spatial resampling is enabled this specifies the size of the
/// encoded frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialResizeParams {
    /// Internal coded frame width.
    pub width: u32,
    /// Internal coded frame height.
    pub height: u32,
    /// Spatial resampling up watermark.
    ///
    /// This threshold is described as a percentage of the target data buffer.
    /// When the data buffer rises above this percentage of fullness, the
    /// encoder will step up to a higher resolution version of the frame.
    pub resize_up_thresh: u32,
    /// Spatial resampling down watermark.
    ///
    /// This threshold is described as a percentage of the target data buffer.
    /// When the data buffer falls below this percentage of fullness, the
    /// encoder will step down to a lower resolution version of the frame.
    pub resize_down_thresh: u32,
}

impl SpatialResizeParams {
    /// Constructs a new set of resize params pre-populated with
    /// the defaults employed by the specific codec.
    pub fn default_for_codec(codec: CodecId) -> Result<Self> {
        let iface = codec.iface()?;
        let mut cfg = unsafe { MaybeUninit::zeroed().assume_init() };
        call_vpx!(
            vpx_sys::vpx_codec_enc_config_default(iface, &mut cfg, 0),
            Error::EncoderConfigInitFailed,
        )?;
        Ok(Self {
            width: cfg.rc_scaled_width,
            height: cfg.rc_scaled_height,
            resize_up_thresh: cfg.rc_resize_up_thresh,
            resize_down_thresh: cfg.rc_resize_down_thresh,
        })
    }
    fn from_cfg(cfg: &vpx_codec_enc_cfg) -> Option<Self> {
        (cfg.rc_resize_allowed != 0).then_some(Self {
            width: cfg.rc_scaled_width,
            height: cfg.rc_scaled_height,
            resize_up_thresh: cfg.rc_resize_up_thresh,
            resize_down_thresh: cfg.rc_resize_down_thresh,
        })
    }
    fn to_cfg(
        params: &Option<SpatialResizeParams>,
        cfg: &mut vpx_codec_enc_cfg,
    ) {
        if let Some(params) = params {
            cfg.rc_resize_allowed = 1;
            cfg.rc_scaled_width = params.width;
            cfg.rc_scaled_height = params.height;
            cfg.rc_resize_up_thresh = params.resize_up_thresh;
            cfg.rc_resize_down_thresh = params.resize_down_thresh;
        } else {
            cfg.rc_resize_allowed = 0;
        }
    }
}

/// Configures the type of rate control the codec should employ.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RateControl {
    /// Constant Bit Rate (CBR) mode, bitrate in kbit/s.
    ConstantBitRate(u32),
    /// Variable Bit Rate (VBR) mode, bitrate in kbit/s.
    VariableBitRate(u32),
    /// Constrained Quality (CQ) mode. This is variable bitrate mode, but
    /// an additional quality cap, so as not to waste bits on scenes where
    /// additional bitrate would be wasted. Use the following control sets
    /// to obtain fine-grained over the bounds of the variable bitrate
    /// algorithm (or just let the codec decide):
    /// - [`ctrl::EncoderControlSet::MaxIntraBitratePct`]
    ///
    /// In addition, for VP9 the following extra sets are available:
    /// - [`ctrl::EncoderControlSet::Vp9MaxInterBitratePct`]
    /// - [`ctrl::EncoderControlSet::Vp9AQMode`]
    /// - [`ctrl::EncoderControlSet::Vp9FramePeriodicBoost`]
    ConstrainedQuality {
        /// Target bitrate in kbit/s.
        bitrate: u32,
        /// Minimum quality setting the encoder should use.
        /// Allowable values 0..63 (lower value = higher quality).
        max_quality: u32,
    },
    /// Constant Quality/Quantizer (Q) mode.
    /// Allowable values 0..63 (lower value = higher quality).
    ConstantQuality(u32),
    /// Lossless quality mode. The decoded bitstream will be an exact
    /// bit-accurate copy of the input. Only supported by the VP9 codec.
    Lossless,
}

/// Stream timebase units.
///
/// Indicates the smallest interval of time, in seconds, used by the stream.
/// For fixed frame rate material, or variable frame rate material where
/// frames are timed at a multiple of a given clock (ex: video capture), the
/// RECOMMENDED method is to set the timebase to the reciprocal of the frame
/// rate (ex: 1001/30000 for 29.970 Hz NTSC). This allows the pts to
/// correspond to the frame number, which can be handy. For re-encoding video
/// from containers with absolute time timestamps, the RECOMMENDED method is
/// to set the timebase to that of the parent container or multimedia
/// framework (ex: 1/1000 for ms, as in FLV).
///
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timebase {
    /// The numerator of the timebase.
    pub num: NonZero<u32>,
    /// The denominator of the timebase.
    pub den: NonZero<u32>,
}

/// Encode Deadline
///
/// The encoder supports the notion of a soft real-time deadline. Given a
/// non-zero value to the deadline parameter (i.e. anything other than
/// `BestQuality`), the encoder will make a "best effort" guarantee to
/// return before the given time slice expires. It is implicit that limiting
/// the available time to encode will degrade the output quality. The encoder
/// can be given an unlimited time to produce the best possible frame by
/// specifying a deadline of `BestQuality`. A `Custom` deadline supersedes
/// the VPx notion of "best quality, good quality, realtime".
///
/// The default is `GoodQuality`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EncodingDeadline {
    /// Deadline indicating that the encoder return as soon as possible
    /// (equal to [`vpx_sys::VPX_DL_REALTIME`] microseconds).
    Realtime,
    /// Deadline equal to [`vpx_sys::VPX_DL_GOOD_QUALITY`] microseconds
    /// (the default).
    #[default]
    GoodQuality,
    /// Deadline equal to [`vpx_sys::VPX_DL_BEST_QUALITY`] (unlimited
    /// encoding time to produce the best frame possible).
    BestQuality,
    /// Custom deadline setting to be passed to the encoder.
    Custom(Duration),
}

impl From<EncodingDeadline> for std::ffi::c_ulong {
    fn from(value: EncodingDeadline) -> Self {
        match value {
            EncodingDeadline::Realtime => vpx_sys::VPX_DL_REALTIME as _,
            EncodingDeadline::GoodQuality => vpx_sys::VPX_DL_GOOD_QUALITY as _,
            EncodingDeadline::BestQuality => vpx_sys::VPX_DL_BEST_QUALITY as _,
            EncodingDeadline::Custom(dl) => dl.as_micros() as _,
        }
    }
}

/// The VP9 codec supports a notion of multiple bitstream profiles.
/// This maps to a set of features that are turned on or off. Often the
/// profile to use is determined by the features of the intended decoder.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum EncoderProfile {
    /// The default bitstream profile for any codec.
    /// - VP8 only supports this basic profile
    /// - VP9 treats this as bitstream Profile 0
    ///
    /// This profile only supports 8-bit 4:2:0 (for both VP8 and VP9).
    #[default]
    Default = 0,
    /// VP 9 Profile 1. 8-bit 4:4:4, 4:2:2, and 4:4:0.
    Vp9Profile1 = 1,
    /// VP9 Profile 2. 10-bit and 12-bit color only, with 4:2:0 sampling.
    Vp9Profile2 = 2,
    /// VP9 Profile 3. 10-bit and 12-bit color only, with 4:2:2/4:4:4/4:4:0
    /// sampling.
    Vp9Profile3 = 3,
}

bitflags! {
    /// These flags define which error resilient features to enable in the
    /// encoder.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ErrorResilient: u32 {
        /// Improve resiliency against losses of whole frames.
        const DEFAULT = vpx_sys::VPX_ERROR_RESILIENT_DEFAULT;
        /// The frame partitions are independently decodable by the bool
        /// decoder, meaning that partitions can be decoded even though
        /// earlier partitions have been lost. Note that intra prediction
        /// is still done over the partition boundary.
        ///
        /// <div class="warning">This is only supported by VP8.</div>
        const RESILIENT_PARTITIONS = vpx_sys::VPX_ERROR_RESILIENT_PARTITIONS;
    }
    /// Encoder configuration flags to enable special global behaviors.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncoderFlags: u32 {
        /// Calculate PSNR on each frame.
        const PSNR = vpx_sys::VPX_CODEC_USE_PSNR;
        /// Make the encoder output one  partition at a time.
        const OUTPUT_PARTITION = vpx_sys::VPX_CODEC_USE_OUTPUT_PARTITION;
        /// Use high bitdepth.
        const HIGH_BIT_DEPTH = vpx_sys::VPX_CODEC_USE_HIGHBITDEPTH;
    }
    /// This type indicates a bitfield to [`Encoder::encode()`], defining
    /// per-frame boolean values.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncoderFrameFlags: u32 {
        /// Force this frame to be a keyframe
        const FORCE_KF = vpx_sys::VPX_EFLAG_FORCE_KF;
        /// Calculate PSNR on this frame, requires
        /// [`EncoderConfig::lag_in_frames`] to be 0.
        const CALCULATE_PSNR = 1 << 1;

        // Caveat: despite these flags having the `VP8_' prefix in libvpx,
        // they seem to still be inspected by the VP9 encoder (seen in
        // `vp9_apply_encoding_flags'), so we drop the algorithm prefix
        // from the names here.

        /// Don't reference the last frame.
        ///
        /// When this flag is set, the encoder will not use the last frame
        /// as a predictor. When not set, the encoder will choose whether
        /// to use the last frame or not automatically.
        const NO_REF_LAST = vpx_sys::VP8_EFLAG_NO_REF_LAST;
        /// Don't reference the golden frame.
        ///
        /// When this flag is set, the encoder will not use the golden frame
        /// as a predictor. When not set, the encoder will choose whether to
        /// use the golden frame or not automatically.
        const NO_REF_GF = vpx_sys::VP8_EFLAG_NO_REF_GF;
        /// Don't reference the alternate reference frame.
        ///
        /// When this flag is set, the encoder will not use the alt ref frame
        /// as a predictor. When not set, the encoder will choose whether to
        /// use the alt ref frame or not automatically.
        const NO_REF_ARF = vpx_sys::VP8_EFLAG_NO_REF_ARF;
        /// Don't update the last frame.
        ///
        /// When this flag is set, the encoder will not update the last frame
        /// with the contents of the current frame.
        const NO_UDP_LAST = vpx_sys::VP8_EFLAG_NO_UPD_LAST;
        /// Don't update the golden frame.
        ///
        /// When this flag is set, the encoder will not update the golden frame
        /// with the contents of the current frame..
        const NO_UDP_GF = vpx_sys::VP8_EFLAG_NO_UPD_GF;
        /// Don't update the alternate reference frame.
        ///
        /// When this flag is set, the encoder will not update the alt ref
        /// frame with the contents of the current frame.
        const NO_UDP_ARF = vpx_sys::VP8_EFLAG_NO_UPD_ARF;
        /// Force golden frame update.
        ///
        /// When this flag is set, the encoder copy the contents of the
        /// current frame to the golden frame buffer.
        const FORCE_GF = vpx_sys::VP8_EFLAG_FORCE_GF;
        /// Force alternate reference frame update.
        ///
        /// When this flag is set, the encoder copy the contents of the
        /// current frame to the alternate reference frame buffer.
        const FORCE_ARF = vpx_sys::VP8_EFLAG_FORCE_ARF;
        /// Disable entropy update.
        ///
        /// When this flag is set, the encoder will not update its internal
        /// entropy model based on the entropy of this frame.
        const NO_UDP_ENTROPY = vpx_sys::VP8_EFLAG_NO_UPD_ENTROPY;
    }
}

/// Keyframe placement mode
///
/// This value indicates whether the encoder should place keyframes at a
/// fixed interval, or determine the optimal placement automatically
/// (as governed by the `min_dist` and `max_dist` parameters)
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyFrameMode {
    /// Encoder determines optimal placement automatically
    Auto {
        /// Keyframe minimum interval
        ///
        /// This value, expressed as a number of frames, prevents the encoder
        /// from placing a keyframe nearer than `min_dist` to the previous
        /// keyframe. At least `min_dist` frames non-keyframes will be coded
        /// before the next keyframe. Set `min_dist` equal to `max_dist` for
        /// a fixed interval.
        min_dist: u32,
        /// Keyframe maximum interval
        ///
        /// This value, expressed as a number of frames, forces the encoder
        /// to code a keyframe if one has not been coded in the last
        /// `max_dist` frames. A value of 0 implies all frames will be
        /// keyframes. Set `min_dist` equal to `max_dist` for a fixed interval.
        max_dist: u32,
    },
    /// Encoder does not place keyframes.
    Disabled,
}

impl KeyFrameMode {
    fn from_cfg(cfg: &vpx_sys::vpx_codec_enc_cfg) -> Self {
        match cfg.kf_mode {
            vpx_sys::vpx_kf_mode::VPX_KF_AUTO => Self::Auto {
                min_dist: cfg.kf_min_dist,
                max_dist: cfg.kf_max_dist,
            },
            // VPX_KF_FIXED is an alias for VPX_KF_DISABLED
            vpx_sys::vpx_kf_mode::VPX_KF_DISABLED => Self::Disabled,
        }
    }
    fn to_cfg(self, cfg: &mut vpx_sys::vpx_codec_enc_cfg) {
        match self {
            KeyFrameMode::Auto { min_dist, max_dist } => {
                cfg.kf_mode = vpx_sys::vpx_kf_mode::VPX_KF_AUTO;
                cfg.kf_min_dist = min_dist;
                cfg.kf_max_dist = max_dist;
            }
            KeyFrameMode::Disabled => {
                cfg.kf_mode = vpx_sys::vpx_kf_mode::VPX_KF_DISABLED;
            }
        }
    }
}

/// Configures the parameters for a spatial coding layer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialLayer {
    /// Enable auto alt reference flags for this spatial layer.
    ///
    /// Defines if auto alt reference frame is enabled for this layer.
    pub enable_auto_alt_ref: bool,

    /// Target bitrate for this spatial layer.
    ///
    /// Defines the target coding bitrate to be used for this layer (in kbps).
    pub target_bitrate: u32,
}

impl SpatialLayer {
    fn from_cfg(cfg: &vpx_sys::vpx_codec_enc_cfg) -> Vec<Self> {
        let mut layers = vec![];
        for i in 0..cfg.ss_number_layers as usize {
            layers.push(SpatialLayer {
                enable_auto_alt_ref: cfg.ss_enable_auto_alt_ref[i] != 0,
                target_bitrate: cfg.ss_target_bitrate[i],
            });
        }
        layers
    }
    fn to_cfg(layers: &[Self], cfg: &mut vpx_sys::vpx_codec_enc_cfg) {
        assert!(layers.len() < vpx_sys::VPX_SS_MAX_LAYERS as usize);
        cfg.ss_number_layers = layers.len() as u32;
        layers.iter().enumerate().for_each(|(i, layer)| {
            cfg.ss_enable_auto_alt_ref[i] = layer.enable_auto_alt_ref as i32;
            cfg.ss_target_bitrate[i] = layer.target_bitrate;
        });
    }
}

/// Configures the parameters for a temporal coding layer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TemporalLayer {
    /// Target bitrate for the temporal layer.
    ///
    /// The target coding bitrate to be used for the temporal layer (in kbps).
    target_bitrate: u32,
    /// Frame rate decimation factor for the temporal layer.
    ///
    /// Specifies the frame rate decimation factor to apply to this layer.
    rate_decimator: u32,
}

impl TemporalLayer {
    fn from_cfg(cfg: &vpx_sys::vpx_codec_enc_cfg) -> Vec<Self> {
        let mut layers = vec![];
        for i in 0..cfg.ts_number_layers as usize {
            layers.push(Self {
                target_bitrate: cfg.ts_target_bitrate[i],
                rate_decimator: cfg.ts_rate_decimator[i],
            })
        }
        layers
    }
    fn to_cfg(layers: &[Self], cfg: &mut vpx_sys::vpx_codec_enc_cfg) {
        assert!(layers.len() < vpx_sys::VPX_TS_MAX_LAYERS as usize);
        cfg.ts_number_layers = layers.len() as u32;
        layers.iter().enumerate().for_each(|(i, layer)| {
            cfg.ts_target_bitrate[i] = layer.target_bitrate;
            cfg.ts_rate_decimator[i] = layer.rate_decimator;
        });
    }
}

/// An iterator which returns packets after a call to
/// [`Encoder::encode()`]. You should drain all packets when the encoder
/// generates them.
pub struct PacketIterator<'a> {
    ctx: &'a mut vpx_sys::vpx_codec_ctx_t,
    iter: vpx_sys::vpx_codec_iter_t,
}

// Safe to send between threads, because we don't hold on to any mutexes
// between function calls and no functions are async.
unsafe impl Send for PacketIterator<'_> {}

impl Iterator for PacketIterator<'_> {
    type Item = Packet;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let pkt = unsafe {
                vpx_sys::vpx_codec_get_cx_data(self.ctx, &mut self.iter)
                    .as_ref()?
            };
            if let Some(pkt) = Packet::from_vpx_packet(pkt) {
                return Some(pkt);
            }
        }
    }
}

/// A packet output from the encoder in response to a call to
/// [`Encoder::encode()`]. The encoded bitstream consists of the
/// contents of compressed frame packets, but the encoder can sometimes
/// also produce some other ancillary information packets. For example,
/// when doing multi-pass encoding, or when running the PSNR analyzer,
/// the encoder will generate packets containing this additional
/// metainformation. In general, though, you only need the packets
/// containing the [`CompressedFrame`] data to decode a video stream.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum Packet {
    /// A compressed video frame.
    CompressedFrame(CompressedFrame),
    /// Two-pass statistics for this frame.
    TwoPassStats(FixedBuffer),
    /// First pass mb statistics for this frame.
    FirstPassMbStats(FixedBuffer),
    /// PSNR statistics for this frame.
    PSNRPacket(PSNRPacket),
    /// Algorithm extensions.
    Custom(FixedBuffer),
}

impl Packet {
    fn from_vpx_packet(pkt: &vpx_sys::vpx_codec_cx_pkt) -> Option<Self> {
        match pkt.kind {
            vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT => {
                let frame = unsafe { &pkt.data.frame };
                if frame.buf.is_null() {
                    return None;
                }
                Some(Self::CompressedFrame(CompressedFrame {
                    data: unsafe {
                        std::slice::from_raw_parts(frame.buf as _, frame.sz)
                    }
                    .to_vec(),
                    pts: frame.pts,
                    // frame.duration is a c_ulong, so need to cast
                    duration: frame.duration as _,
                    flags: CompressedFrameFlags::from(frame.flags),
                    partition_id: frame.partition_id,
                    width: frame.width,
                    height: frame.height,
                    spatial_layer_encoded: frame.spatial_layer_encoded,
                }))
            }
            vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_STATS_PKT => {
                Some(Self::TwoPassStats(FixedBuffer::from_vpx(unsafe {
                    &pkt.data.twopass_stats
                })?))
            }
            vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_FPMB_STATS_PKT => {
                Some(Self::FirstPassMbStats(FixedBuffer::from_vpx(unsafe {
                    &pkt.data.firstpass_mb_stats
                })?))
            }
            vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_PSNR_PKT => {
                let psnr = unsafe { &pkt.data.psnr };
                Some(Self::PSNRPacket(PSNRPacket {
                    samples: PSNRSamples {
                        total: psnr.samples[0],
                        y: psnr.samples[1],
                        u: psnr.samples[2],
                        v: psnr.samples[3],
                    },
                    sse: PSNRSumSquaredError {
                        total: psnr.sse[0],
                        y: psnr.sse[1],
                        u: psnr.sse[2],
                        v: psnr.sse[3],
                    },
                    psnr: PSNR {
                        total: psnr.psnr[0],
                        y: psnr.psnr[1],
                        u: psnr.psnr[2],
                        v: psnr.psnr[3],
                    },
                }))
            }
            _ => Some(Self::Custom(FixedBuffer::from_vpx(unsafe {
                &pkt.data.raw
            })?)),
        }
    }
}

/// A compressed frame produced by the encoder.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompressedFrame {
    /// Compressed data of the frame. This is what you need to store or
    /// send over the wire, in order to produce a decodable bistream.
    pub data: Vec<u8>,
    /// Presentation timestamp (in timebase units).
    pub pts: i64,
    /// Duration in timebase units.
    pub duration: u64,
    /// Frame flags.
    pub flags: CompressedFrameFlags,
    /// The partition id defines the decoding order of the partitions.
    /// Only applicable when "output partition" mode is enabled. First
    /// partition has id 0.
    pub partition_id: i32,
    /// Width of frames in this packet. VP8 will only use the first one.
    pub width: [u32; vpx_sys::VPX_SS_MAX_LAYERS as usize],
    /// Height of frames in this packet. VP8 will only use the first one.
    pub height: [u32; vpx_sys::VPX_SS_MAX_LAYERS as usize],
    /// Flag to indicate if spatial layer frame in this packet is encoded
    /// or dropped. VP8 will always be set to 1.
    pub spatial_layer_encoded: [u8; vpx_sys::VPX_SS_MAX_LAYERS as usize],
}

/// Flags pertaining to an encoded frame.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompressedFrameFlags {
    /// Frame is the start of a Group Of Pictures (GOP), i.e. a keyframe.
    pub is_key: bool,
    /// Frame can be dropped without affecting the stream (no future frame
    /// depends on this one).
    pub is_droppable: bool,
    /// Frame should be decoded but will not be shown.
    pub is_invisible: bool,
    /// This is a fragment of the encoded frame.
    pub is_fragment: bool,
}

impl From<vpx_sys::vpx_codec_frame_flags_t> for CompressedFrameFlags {
    fn from(value: vpx_sys::vpx_codec_frame_flags_t) -> Self {
        Self {
            is_key: (value & vpx_sys::VPX_FRAME_IS_KEY) != 0,
            is_droppable: (value & vpx_sys::VPX_FRAME_IS_DROPPABLE) != 0,
            is_invisible: (value & vpx_sys::VPX_FRAME_IS_INVISIBLE) != 0,
            is_fragment: (value & vpx_sys::VPX_FRAME_IS_FRAGMENT) != 0,
        }
    }
}

/// An opaque buffer representing arbitrary data given to us by libvpx
/// in a returned Packet. You can access the underlying data by calling
/// the unsafe [`FixedBuffer::get_inner()`] method with the appropriate
/// return type. If the libvpx buffer is null, this type will never be
/// constructed, so if you see a `FixedBuffer`, you can be sure that
/// there is *something* there.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FixedBuffer(Vec<u8>);

impl FixedBuffer {
    /// Unsafely casts the pointer contained in the fixed buffer to a
    /// reference of the return type of this function.
    ///
    /// # Safety
    ///
    /// We cannot fully check what type the contained data is, so it is up
    /// to the caller to make sure they're only grabbing the correct type
    /// here. This function only checks that the size of the buffer target
    /// matches the return type's size.
    pub unsafe fn inner<T: Sized>(&self) -> Option<&T> {
        if self.0.len() != size_of::<T>() {
            return None;
        }
        unsafe { (self.0.as_ptr() as *const T).as_ref() }
    }
    fn from_vpx(value: &vpx_sys::vpx_fixed_buf_t) -> Option<Self> {
        let buf = unsafe {
            std::slice::from_raw_parts(
                std::ptr::NonNull::new(value.buf)?.as_ptr() as *const u8,
                value.sz,
            )
            .to_vec()
        };

        Some(Self(buf))
    }
}

/// PSNR statistics for a frame.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct PSNRPacket {
    /// Number of samples.
    pub samples: PSNRSamples,
    /// Sum squared error.
    pub sse: PSNRSumSquaredError,
    /// PSNR.
    pub psnr: PSNR,
}

/// PSNR stats samples.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct PSNRSamples {
    pub total: u32,
    pub y: u32,
    pub u: u32,
    pub v: u32,
}

/// PSNR sum squared error.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct PSNRSumSquaredError {
    pub total: u64,
    pub y: u64,
    pub u: u64,
    pub v: u64,
}

/// Perceptual Signal-to-Noise Ratio
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct PSNR {
    pub total: f64,
    pub y: f64,
    pub u: f64,
    pub v: f64,
}

#[cfg(test)]
mod test {
    use super::{
        CodecId, CompressedFrame, CompressedFrameFlags, Encoder, EncoderConfig,
        EncoderFlags, EncoderFrameFlags, ErrorResilient, FixedBuffer,
        KeyFrameMode, PSNRPacket, PSNRSamples, PSNRSumSquaredError,
        PacketIterator, RateControl, SpatialLayer, SpatialResizeParams,
        TemporalLayer, Timebase, PSNR,
    };
    #[test]
    fn test_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CodecId>();
        assert_send::<CompressedFrame>();
        assert_send::<CompressedFrameFlags>();
        assert_send::<Encoder<u8>>();
        assert_send::<EncoderConfig<u8>>();
        assert_send::<EncoderFlags>();
        assert_send::<EncoderFrameFlags>();
        assert_send::<ErrorResilient>();
        assert_send::<FixedBuffer>();
        assert_send::<KeyFrameMode>();
        assert_send::<PacketIterator>();
        assert_send::<PSNR>();
        assert_send::<PSNRPacket>();
        assert_send::<PSNRSamples>();
        assert_send::<PSNRSumSquaredError>();
        assert_send::<RateControl>();
        assert_send::<SpatialLayer>();
        assert_send::<SpatialResizeParams>();
        assert_send::<TemporalLayer>();
        assert_send::<Timebase>();
    }
    #[test]
    fn test_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<CodecId>();
        assert_sync::<CompressedFrame>();
        assert_sync::<CompressedFrameFlags>();
        assert_sync::<Encoder<u8>>();
        assert_sync::<EncoderConfig<u8>>();
        assert_sync::<EncoderFlags>();
        assert_sync::<EncoderFrameFlags>();
        assert_sync::<ErrorResilient>();
        assert_sync::<FixedBuffer>();
        assert_sync::<KeyFrameMode>();
        assert_sync::<PSNR>();
        assert_sync::<PSNRPacket>();
        assert_sync::<PSNRSamples>();
        assert_sync::<PSNRSumSquaredError>();
        assert_sync::<RateControl>();
        assert_sync::<SpatialLayer>();
        assert_sync::<SpatialResizeParams>();
        assert_sync::<TemporalLayer>();
        assert_sync::<Timebase>();
    }
}
