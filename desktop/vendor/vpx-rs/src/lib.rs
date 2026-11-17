/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

#![warn(missing_docs)]

//! This crate provides a Rusty interface to Google's libvpx library
//! for handling VP8 and VP9 video streams. The low-level bindings to
//! libvpx are provided by the
//! [env-libvpx-sys](https://crates.io/crates/env-libvpx-sys) crate,
//! which this crate then wraps in Rust types, hopefully making the
//! experience a bit neater and less error-prone.
//!
//! While an effort was made to hopefully cover most usecases of libvpx,
//! this crate doesn't cover *all* of libvpx's API, as some of it is
//! rather esoteric and difficult to make feel natural and safe to use
//! in Rust. If you find a particular set of features of libvpx's API
//! is missing, please contact the crate's author.
//!
//! # Usage Examples
//!
//! Refer to the [`Encoder`] and [`Decoder`] structs for examples on
//! how to use the library to encode and decode video frames.
//!
//! The library supports a variety of YUV image formats. Have a look at
//! [`ImageFormat`] for more information on the exact list, as well as
//! tips on how to convert between YUV and non-YUV formats.

/// Common types and structs shared between the encoder, decoder and
/// image processing portions of the crate.
pub mod common;

/// Contains the decoding portion of the library. See [`Decoder`] for
/// a simple example on how to decode images.
pub mod dec;

/// Contains the encoding portion of the library. See [`Encoder`] for
/// a simple example on how to encode images.
pub mod enc;

/// Contains definitions for the various YUV image formats supported by
/// the crate. You should take a look at [`YUVImageData`] especially,
/// since it is what the [`Encoder`] expects as its input. The list of
/// supported formats is detailed in [`ImageFormat`].
pub mod image;

pub use crate::common::StreamInfo;
pub use crate::dec::{DecodedImage, DecodedImageData, Decoder, DecoderConfig};
pub use crate::enc::{
    CompressedFrame, Encoder, EncoderConfig, EncoderFlags, EncoderFrameFlags,
    EncodingDeadline, Packet, RateControl, Timebase,
};
pub use crate::image::{ImageFormat, YUVImageData, YUVImageDataOwned};

// Re-export the vpx_sys crate, because the low-level bindings are used
// in some calls in enc::ctrl.
pub use vpx_sys;

#[doc(hidden)]
#[macro_export]
macro_rules! call_vpx_ptr {
    ($e: expr, $errenum: expr $(,)?) => {{
        #[allow(clippy::macro_metavars_in_unsafe)]
        let retval = unsafe { $e };
        if !retval.is_null() {
            Ok(retval)
        } else {
            Err($errenum)
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! call_vpx {
    ($e: expr, $errenum: expr $(,)?) => {{
        #[allow(clippy::macro_metavars_in_unsafe)]
        let err: vpx_sys::vpx_codec_err_t = unsafe { $e };
        if err == vpx_sys::VPX_CODEC_OK {
            Ok(())
        } else {
            Err($errenum(err, unsafe {
                std::ffi::CStr::from_ptr(vpx_sys::vpx_codec_err_to_string(err))
                    .to_string_lossy()
                    .to_string()
            }))
        }
    }};
}

/// Error enums returned by this crate. Many variants include a
/// `vpx_codec_err_t` - this encodes the underlying `libvpx` library
/// error code. You can use this to fine-tune error reporting if you
/// so desire. The included string is a human-readable representation
/// of the error code as returned from libvpx.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq, Hash)]
pub enum Error {
    /// An attempt to instantiate a given codec interface failed,
    /// probably because libvpx was compiled with that codec disabled.
    #[error("Codec interface not available")]
    CodecInterfaceNotAvailable,
    /// A call to [`Encoder::codec_control_set()`] failed. The attached
    /// libvpx error code might tell you more information about why the
    /// call failed (usually this is due to an argument value being invalid).
    #[error("Codec control call failed: {0:?}: {1}")]
    CodecControlFailed(vpx_sys::vpx_codec_err_t, String),
    /// There was an error initializing the encoder configuration.
    #[error("Failed to initialize the encoder configuration: {0:?}: {1}")]
    EncoderConfigInitFailed(vpx_sys::vpx_codec_err_t, String),
    /// Failed to initialize encoder or decoder context.
    #[error("Codec initialization failed with code {0:?}: {1}")]
    VpxCodecInitFailed(vpx_sys::vpx_codec_err_t, String),
    /// The specified rate control mode (lossless) isn't supported by the
    /// codec (VP8).
    #[error("Unsupported rate control mode for this codec")]
    UnsupportedRateControlMode,
    /// libvpx returned an error trying to wrap an image for encoding.
    #[error("Error wrapping YUV image")]
    ErrorWrappingImage,
    /// libvpx returned an error trying to encode the image we passed
    /// it in [`Encoder::encode()`].
    #[error("VPX encode failed with {0:?}: {1}")]
    ErrorEncodingImage(vpx_sys::vpx_codec_err_t, String),
    /// libvpx returned an error trying to decode the bitstream we passed
    /// it in [`Decoder::decode()`].
    #[error("Error decoding bitstream: {0:?}: {1}")]
    ErrorDecodingBitstream(vpx_sys::vpx_codec_err_t, String),
    /// We've encountered an invalid image format passed to us from libvpx.
    #[error("Invalid image format ({0:?})")]
    InvalidImageFormat(vpx_sys::vpx_img_fmt),
    /// The raw image data buffer has the wrong length for the specified
    /// image format, width and height (either too little or too much data
    /// compared to what a well-formed data buffer should have).
    #[error(
        "Image data has bad length, expected: {expected}, received: {received}"
    )]
    ImageDataBadLength {
        /// Expected data buffer length (in samples, **not** bytes).
        expected: usize,
        /// Actual data buffer length (in samples, **not** bytes).
        received: usize,
    },
    /// The image width wasn't a multiple of 2, but the passed image format
    /// requires it to be.
    #[error("Image width isn't a multiple of 2")]
    WidthNotMultipleOfTwo,
    /// The image height wasn't a multiple of 2, but the passed image format
    /// requires it to be.
    #[error("Image height isn't a multiple of 2")]
    HeightNotMultipleOfTwo,
    /// Indicates that an attempt was made to set anything other than the
    /// default profile for VP8 (which only supports that).
    #[error("Invalid profile selected for codec")]
    InvalidProfileSelected,
}

/// Result type for this crate.
pub type Result<T> = ::core::result::Result<T, Error>;

pub(crate) struct NoDebugWrapper<T: 'static>(T);

impl<T: 'static> std::fmt::Debug for NoDebugWrapper<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::any::TypeId::of::<T>().fmt(f)
    }
}
