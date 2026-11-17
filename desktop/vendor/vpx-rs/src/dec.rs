/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

use bitflags::bitflags;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use crate::common::StreamInfo;
use crate::image::YUVImageData;
use crate::NoDebugWrapper;
use crate::{call_vpx, call_vpx_ptr};
use crate::{Error, Result};

#[allow(missing_docs)]
/// Specifies the codec to be used for decoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CodecId {
    VP8,
    VP9,
}

impl CodecId {
    pub(crate) fn iface(&self) -> Result<*const vpx_sys::vpx_codec_iface_t> {
        match self {
            Self::VP8 => call_vpx_ptr!(
                vpx_sys::vpx_codec_vp8_dx(),
                Error::CodecInterfaceNotAvailable,
            ),
            Self::VP9 => call_vpx_ptr!(
                vpx_sys::vpx_codec_vp9_dx(),
                Error::CodecInterfaceNotAvailable,
            ),
        }
    }
}

/// The main VP8/VP9 video decoder.
///
/// # Example
/// ```
/// use std::num::NonZero;
/// use vpx_rs::{Decoder, DecoderConfig, Error, DecodedImageData};
///
/// // Initialize a new decoder
/// fn new_vp8_decoder(width: u32, height: u32,) -> Result<Decoder, Error> {
///     // Prepare the configuration
///     let config = DecoderConfig::new(
///         vpx_rs::dec::CodecId::VP8,
///         width,
///         height,
///     );
///     // Instantiate the decoder
///     Decoder::new(config)
/// }
///
/// // Passes a buffer to the decoder and extracts any completed frames
/// fn decode_vp8_buf(
///     decoder: &mut Decoder,
///     stream_data: &[u8],
/// ) -> Result<(), Error> {
///     for decoded_image in decoder.decode(stream_data)?.into_iter() {
///         let (format, bit_depth) = match decoded_image.data() {
///             DecodedImageData::Data8b(data) =>
///                 (format!("{:?}", data.format()), 8),
///             DecodedImageData::Data16b(data) =>
///                 (format!("{:?}", data.format()), 16),
///         };
///         println!(
///             "Decoded a {}x{} pixel image in {} format with bit depth {}",
///             decoded_image.width(),
///             decoded_image.height(),
///             format,
///             bit_depth,
///         );
///     }
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct Decoder {
    ctx: NoDebugWrapper<vpx_sys::vpx_codec_ctx_t>,
}

// We are Send-safe, because we don't hold onto any mutexes between calls
// and no functions are async. Also, we have no interior mutability through
// immutable references.
unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

impl Decoder {
    /// Constructs a new decoder for the given configuration. After
    /// successful construction, you can start sending bitstream buffers
    /// to the decoder by calling [`Decoder::decode()`].
    pub fn new(config: DecoderConfig) -> Result<Self> {
        let mut ctx = unsafe { MaybeUninit::zeroed().assume_init() };
        let (iface, cfg) = config.cfg()?;
        call_vpx!(
            vpx_sys::vpx_codec_dec_init_ver(
                &mut ctx,
                iface,
                &cfg,
                config.flags.bits() as _,
                vpx_sys::VPX_DECODER_ABI_VERSION as _,
            ),
            Error::VpxCodecInitFailed,
        )?;
        Ok(Self {
            ctx: NoDebugWrapper(ctx),
        })
    }
    /// Submits data to be decoded by the decoder, yielding a number of
    /// decoded images as a result. Please note that even if you only send
    /// data for a single frame at a time, due to internal bitstream frame
    /// rendering, the decoder might produce any number of frames per
    /// `decode` call. You should retrieve all decoded images from the
    /// returned iterator before trying to push more data to the decoder.
    /// The images will always be returned to you in presentation order,
    /// so there's no need to perform image reordering on your end.
    pub fn decode(&mut self, data: &[u8]) -> Result<DecodedImageIterator> {
        call_vpx!(
            vpx_sys::vpx_codec_decode(
                &mut self.ctx.0,
                data.as_ptr(),
                data.len() as _,
                std::ptr::null_mut(),
                0,
            ),
            Error::ErrorDecodingBitstream,
        )?;
        Ok(DecodedImageIterator {
            ctx: &mut self.ctx.0,
            iter: std::ptr::null_mut(),
        })
    }
    /// Returns stream information for the decoder. This might not return
    /// stream information initially, especially before sending any data
    /// to the decoder.
    pub fn stream_info(&mut self) -> Option<StreamInfo> {
        crate::common::get_stream_info(&mut self.ctx.0)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // We unwrap here, because the only documented cases for this to fail
        // are when passing a null pointer (which we cannot do) and the context
        // not being initialized (which by design cannot happen in Rust).
        unsafe {
            let error = vpx_sys::vpx_codec_destroy(&mut self.ctx.0);
            assert_eq!(error, vpx_sys::VPX_CODEC_OK);
        }
    }
}

/// The configuration for a new decoder. You must pass this to
/// [`Decoder::new()`].
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecoderConfig {
    /// The codec to initialize in the decoder.
    pub codec: CodecId,
    /// The initial width of the decoder in pixels. Please note that
    /// VPx streams can change resolution dynamically, so this is only
    /// an initial hint to the decoder and you should look at the
    /// decoded images to follow a potential resolution change.
    pub width: u32,
    /// The initial height of the decoder in pixels. Please note that
    /// VPx streams can change resolution dynamically, so this is only
    /// an initial hint to the decoder and you should look at the
    /// decoded images to follow a potential resolution change.
    pub height: u32,
    /// Maximum number of threads to use, default 1. Zero is ignored and
    /// treated as 1.
    pub threads: u32,
    /// Flags to set on the decoder.
    pub flags: DecoderFlags,
}

impl DecoderConfig {
    /// Constructs a new decoder configuration.
    /// Note: `width` and `height` are in pixels.
    pub fn new(codec: CodecId, width: u32, height: u32) -> Self {
        Self {
            codec,
            width,
            height,
            threads: 1,
            flags: DecoderFlags::default(),
        }
    }
    fn cfg(
        &self,
    ) -> Result<(*const vpx_sys::vpx_codec_iface, vpx_sys::vpx_codec_dec_cfg)>
    {
        Ok((
            self.codec.iface()?,
            vpx_sys::vpx_codec_dec_cfg {
                w: self.width,
                h: self.height,
                threads: self.threads,
            },
        ))
    }
}

bitflags! {
    /// Configuration flags for the decoder.
    #[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
    pub struct DecoderFlags: u32 {
        /// Postprocess the decoded frame
        const POSTPROC = vpx_sys::VPX_CODEC_USE_POSTPROC;
        /// Conceal errors in decoded frames
        const ERROR_CONCEALMENT = vpx_sys::VPX_CODEC_USE_ERROR_CONCEALMENT;
        /// The input frame should be passed to the decoder one fragment
        /// at a time
        const INPUT_FRAGMENTS = vpx_sys::VPX_CODEC_USE_INPUT_FRAGMENTS;
        /// Enable frame-based multi-threading
        const FRAME_THREADING = vpx_sys::VPX_CODEC_USE_FRAME_THREADING;
    }
}

/// An iterator returned from a call to [`Decoder::decode`]. Due to frame
/// reordering, the decoder can return any number of frames for a single
/// `decode` call. You should retrieve all decoded images from this iterator
/// before trying to push more data to the decoder. The images will always
/// be returned to you in presentation order, so there's no need to perform
/// image reordering on your end.
pub struct DecodedImageIterator<'a> {
    ctx: &'a mut vpx_sys::vpx_codec_ctx,
    iter: vpx_sys::vpx_codec_iter_t,
}

// This is send-safe, because we don't hold onto any mutexes between calls
// and none of our functions are async. Also, we have no interior mutability
// through immutable references, so we're Sync-safe.
unsafe impl Send for DecodedImageIterator<'_> {}
unsafe impl Sync for DecodedImageIterator<'_> {}

impl Iterator for DecodedImageIterator<'_> {
    type Item = DecodedImage;
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let img = vpx_sys::vpx_codec_get_frame(self.ctx, &mut self.iter);
            Some(DecodedImage {
                img: NonNull::new(img)?,
            })
        }
    }
}

/// Data structure owning a decoded image from a VPx video stream.
/// Call [`DecodedImage::data()`] to access the image data.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct DecodedImage {
    // We own the vpx_image, so during drop, we free the image.
    img: NonNull<vpx_sys::vpx_image>,
}

// A decoded image is both send and sync safe, because it is completely
// owned by us and holds no mutexes for access.
unsafe impl Send for DecodedImage {}
unsafe impl Sync for DecodedImage {}

/// Encapsulates the specific pixel type returned in the decoded image.
/// libvpx will return [`u16`] pixel data from high-bit-depth (10-bit
/// and 12-bit) video streams.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecodedImageData<'a> {
    /// Underlying data is 8-bits per sample. This variant is generated
    /// for a normal bit depth (8-bit) video stream.
    Data8b(YUVImageData<'a, u8>),
    /// Underlying data is 16-bits per sample. This variant is generated
    /// for a high bit depth (10-bit/12-bit) video stream.
    Data16b(YUVImageData<'a, u16>),
}

impl DecodedImage {
    /// Returns the color space of the decoded image.
    pub fn color_space(&self) -> vpx_sys::vpx_color_space {
        unsafe { self.img.as_ref().cs }
    }
    /// Returns the display width of the decoded image in pixels.
    pub fn width(&self) -> u32 {
        unsafe { self.img.as_ref().d_w }
    }
    /// Returns the display height of the decoded image in pixels.
    pub fn height(&self) -> u32 {
        unsafe { self.img.as_ref().d_h }
    }
    /// Allows access to the decoded image data.
    pub fn data(&self) -> DecodedImageData {
        unsafe {
            use vpx_sys::vpx_img_fmt::*;
            let img = self.img.as_ref();
            match img.fmt {
                VPX_IMG_FMT_YV12 | VPX_IMG_FMT_NV12 | VPX_IMG_FMT_I420
                | VPX_IMG_FMT_I422 | VPX_IMG_FMT_I444 | VPX_IMG_FMT_I440 => {
                    DecodedImageData::Data8b(
                        YUVImageData::<u8>::from_vpx_image(img),
                    )
                }
                VPX_IMG_FMT_I42016 | VPX_IMG_FMT_I42216
                | VPX_IMG_FMT_I44416 | VPX_IMG_FMT_I44016 => {
                    DecodedImageData::Data16b(
                        YUVImageData::<u16>::from_vpx_image(img),
                    )
                }
                _ => unreachable!(
                    "libvpx returned invalid image format: {:?}",
                    img.fmt,
                ),
            }
        }
    }
}

impl Drop for DecodedImage {
    fn drop(&mut self) {
        unsafe { vpx_sys::vpx_img_free(self.img.as_ptr()) }
    }
}

#[cfg(test)]
mod test {
    use super::{
        DecodedImage, DecodedImageIterator, Decoder, DecoderConfig,
        DecoderFlags,
    };
    #[test]
    fn test_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Decoder>();
        assert_send::<DecoderConfig>();
        assert_send::<DecoderFlags>();
        assert_send::<DecodedImage>();
        assert_send::<DecodedImageIterator>();
    }
    #[test]
    fn test_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<Decoder>();
        assert_sync::<DecoderConfig>();
        assert_sync::<DecoderFlags>();
        assert_sync::<DecodedImage>();
        assert_sync::<DecodedImageIterator>();
    }
}
