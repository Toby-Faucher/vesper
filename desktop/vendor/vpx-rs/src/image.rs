/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

use std::mem::MaybeUninit;

use crate::{Error, Result};

/// This trait identifies pixel data types which this crate can work with.
/// At this time, libvpx only supports 8- or 16-bit data, so this trait is
/// implemented only for [`u8`] and [`u16`]'. These are the data types you
/// use when constructing a [`crate::EncoderConfig`] and [`crate::Encoder`]'.
pub trait YUVPixelType:
    Sized + Copy + Default + PartialEq + Eq + 'static + private::Sealed
{
}

impl YUVPixelType for u8 {}
impl YUVPixelType for u16 {}

mod private {
    // Prevent external crates from implementing YUVPixelType for anything
    // other than what libvpx can handle.
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// Defines an image format which the library can process. All formats
/// have the first pixel at the top left of the image and then are ordered
/// left-to-right in rows, and rows are in sequence from top-to-bottom.
///
/// <div class="warning">
///
/// For formats which support 16-bit samples, the samples must be in
/// **native** byte order.
///
/// </div>
///
/// # Image Format Conversion
///
/// If you need to convert between different image formats, especially
/// moving data between RGB(A) and YUV, have a look at the
/// [yuv](https://docs.rs/yuv/latest/yuv/) crate. This crate contains
/// many highly optimized conversion functions that make this a fairly
/// straightforward process.
///
/// ### Converting from RGB to YUV using the [yuv](https://docs.rs/yuv/latest/yuv/) crate
///
/// ```
/// extern crate yuv;
/// use vpx_rs::{ImageFormat, YUVImageDataOwned};
/// use std::marker::PhantomData;
///
/// fn convert_rgb_for_encoder(
///     width: u32,
///     height: u32,
///     input_rgb: &[u8]
/// ) -> YUVImageDataOwned<u8> {
///     let mut buf = vec![];
///     // YUV 4:2:0 is a very common image format for video compression
///     yuv::rgb_to_yuv420(
///         &mut wrap_yuv420_buf(width as usize, height as usize, &mut buf),
///         input_rgb,
///         width * 3,
///         yuv::YuvRange::Full,
///         yuv::YuvStandardMatrix::Bt601,
///         yuv::YuvConversionMode::Balanced,
///     )
///     .expect("YUV conversion failed");
///     YUVImageDataOwned::from_raw_data_vec(
///         ImageFormat::I420,
///         width as usize,
///         height as usize,
///         buf,
///     )
///     .expect("Failed to create image data")
/// }
///
/// fn wrap_yuv420_buf(
///     width: usize,
///     height: usize,
///     outbuf: &mut Vec<u8>,
/// ) -> yuv::YuvPlanarImageMut<'_, u8> {
///     let bufsz = ImageFormat::I420
///         .buffer_len(width, height)
///         .unwrap_or_else(|_| panic!("Invalid {width} or {height}"));
///     outbuf.resize(bufsz, 0);
///     // Split the output buffer into Y, U and V at the plane boundaries
///     let (y_plane, uv_plane) = outbuf.split_at_mut(width * height);
///     let (u_plane, v_plane) =
///         uv_plane.split_at_mut((width / 2) * (height / 2));
///     yuv::YuvPlanarImageMut {
///         y_plane: yuv::BufferStoreMut::Borrowed(y_plane),
///         y_stride: width as u32,
///         u_plane: yuv::BufferStoreMut::Borrowed(u_plane),
///         u_stride: (width / 2) as u32,
///         v_plane: yuv::BufferStoreMut::Borrowed(v_plane),
///         v_stride: (width / 2) as u32,
///         width: width as u32,
///         height: height as u32,
///     }
/// }
/// ```
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageFormat {
    /// YVU 4:2:0 planar format, with the following image planes one after
    /// the other:
    /// |<div style="width: 400px">Luma Y</div>| Chroma V | Chroma U |
    /// |--------------------------------------|----------|----------|
    /// | Full resolution |Half resolution on X & Y|Half resolution on X & Y|
    /// 1. Y (luma) at full resolution
    /// 2. V (chroma) at half resolution both horizontally and vertically
    /// 3. U (chroma) at half resolution both horizontally and vertically
    /// - This format reruires that both the horizontal and vertical
    ///   resolution **must** be a multiple of 2.
    /// - Samples must be [`u8`].
    ///
    /// <div class="warning">
    ///
    /// While similar to the I420/YUV420 format, this
    /// format has the U and V planes reversed. In YV12, the V plane comes
    /// before the U plane. When dealing with YUV 420 data (the most typical),
    /// you'll probably want the `I420` format described
    /// below.
    ///
    /// </div>
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    ///             +-------------------------------+-------+-------+
    ///             |          Luma Y plane         |ChromaV|ChromaU|
    ///             |          (16 Samples)         |(4Smpl)|(4Smpl)|
    ///  Sample     +-------------------------------+-------+-------+
    /// sequence -> |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|V|V|V|V|U|U|U|U|
    ///             +-------------------------------+-------+-------+
    ///             | row 1 | row 2 | row 3 | row 4 |row|row|row|row|
    ///                                             | 1 | 2 | 1 | 2 |
    /// </pre>
    YV12,
    /// YUV 4:2:0 bi-planar format, with the following planes in sequence:
    /// |<div style="width: 400px">Luma Y</div>| Chroma interleaved U & V |
    /// |--------------------------------------|--------------------------|
    /// | Full resolution |Half resolution on X & Y, samples interleaved  |
    /// 1. Y (luma) at full resolution, 8-bits per sample
    /// 2. U and V (chroma) at half resolution both vertically and
    ///    horizontally. The pixel samples are *interleaved* in a single
    ///    chroma plane.
    /// - This format requires that both the horizontal and vertical
    ///   resolution **must** be a multiple of 2.
    /// - Samples must be [`u8`].
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    ///             +-------------------------------+-------+-------+
    ///             |          Luma Y plane         | Chroma U & V  |
    ///             |          (16 Samples)         |  (8 Samples)  |
    ///  Sample     +-------------------------------+---------------+
    /// sequence -> |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|U|V|U|V|U|V|U|V|
    ///             +-------------------------------+---------------+
    ///             | row 1 | row 2 | row 3 | row 4 | row 1 | row 2 |
    /// </pre>
    NV12,
    /// YUV 4:2:0 planar format, with the following planes in sequence:
    /// 1. Y (luma) at full resolution, 8 bits per sample
    /// 2. U (chroma) at half resolution both horizontally and vertically
    /// 3. V (chroma) at half resolution both horizontally and vertically
    /// - This format requires that both the horizontal and vertical
    ///   resolution **must** be a multiple of 2.
    /// - Samples can be [`u8`] or [`u16`] (though [`u8`] is most typical).
    /// - This is the format you will most typically encounter when dealing
    ///   with YUV video data.
    ///
    /// <div class="warning">
    ///
    /// While similar to the `YV12` format, this format has the U and
    /// planes reversed. In `I420`, the U plane comes before the V plane.
    ///
    /// </div>
    ///
    /// <div class="warning">
    ///
    /// This is the most typically encountered planar YUV format, often
    /// referred to as `YUV420`. The
    /// [yuv](https://docs.rs/yuv/latest/yuv/#functions) crate functions
    /// with `yuv420` in their name convert into and out of this format.
    ///
    /// </div>
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    ///             +-------------------------------+-------+-------+
    ///             |          Luma Y plane         |ChromaU|ChromaV|
    ///             |          (16 Samples)         |(4Smpl)|(4Smpl)|
    ///  Sample     +-------------------------------+-------+-------+
    /// sequence -> |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|U|U|U|U|V|V|V|V|
    ///             +-------------------------------+-------+-------+
    ///             | row 1 | row 2 | row 3 | row 4 |row|row|row|row|
    ///                                             | 1 | 2 | 1 | 2 |
    /// </pre>
    I420,
    /// Similar to `I420`, but chroma resolution is halved only on the
    /// horizontal axis. Thus, only the horizontal resolution must be a
    /// multiple of 2. This format is commonly also known as YUV 4:2:2.
    ///
    /// Samples can be [`u8`] or [`u16`].
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    /// +-------------------------------+---------------+---------------+
    /// |          Luma Y plane         |   Chroma U    |   Chroma V    |
    /// |          (16 Samples)         |  (8 Samples)  |  (8 Samples)  |
    /// +-------------------------------+---------------+---------------+
    /// |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|U|U|U|U|U|U|U|U|V|V|V|V|V|V|V|V|
    /// +-------------------------------+---------------+---------------+
    /// | row 1 | row 2 | row 3 | row 4 |row|row|row|row|row|row|row|row|
    ///                                 | 1 | 2 | 3 | 4 | 1 | 2 | 3 | 4 |
    /// </pre>
    I422,
    /// Similar to `I420`, but the chroma planes are at full resolution on
    /// both axes. This format places no constraints on the horizontal or
    /// vertical resolution. This format is also commonly known as YUV 4:4:4.
    ///
    /// Samples can be [`u8`] or [`u16`].
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    /// +-------------------------------+-------------------------------+-------------------------------+
    /// |          Luma Y plane         |        Chroma U plane         |          Chroma V plane       |
    /// |          (16 Samples)         |         (16 Samples)          |           (16 Samples)        |
    /// +-------------------------------+-------------------------------+-------------------------------+
    /// |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|U|U|U|U|U|U|U|U|U|U|U|U|U|U|U|U|V|V|V|V|V|V|V|V|V|V|V|V|V|V|V|V|
    /// +-------------------------------+-------------------------------+-------------------------------+
    /// | row 1 | row 2 | row 3 | row 4 | row 1 | row 2 | row 3 | row 4 | row 1 | row 2 | row 3 | row 4 |
    /// </pre>
    I444,
    /// Similar to `I422`, but chroma resolution is halved on the vertical
    /// instead of the horizontal axis. Thus, only the vertical resolution
    /// must be a multiple of 2. This format is also commonly known as YUV
    /// 4:4:0.
    ///
    /// Samples can be [`u8`] or [`u16`].
    ///
    /// Example 4x4 pixel image:
    /// <pre>
    /// +-------------------------------+---------------+---------------+
    /// |          Luma Y plane         |   Chroma U    |   Chroma V    |
    /// |          (16 Samples)         |  (8 Samples)  |  (8 Samples)  |
    /// +-------------------------------+---------------+---------------+
    /// |Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|Y|U|U|U|U|U|U|U|U|V|V|V|V|V|V|V|V|
    /// +-------------------------------+---------------+---------------+
    /// | row 1 | row 2 | row 3 | row 4 | row 1 | row 2 | row 1 | row 2 |
    /// </pre>
    I440,
}

#[inline]
fn check_dim_mult_2(dim: usize, err: Error) -> Result<()> {
    if dim % 2 == 0 {
        Ok(())
    } else {
        Err(err)
    }
}

impl ImageFormat {
    /// Converts from a libvpx image format to an [`ImageFormat`] enum.
    /// Returns `None` if the libvpx image format is unsupported or invalid.
    pub(crate) fn from_vpx<T: YUVPixelType>(
        value: vpx_sys::vpx_img_fmt,
    ) -> Option<Self> {
        let t = std::any::TypeId::of::<T>();
        match value {
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_YV12 => Some(Self::YV12),
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_NV12 => Some(Self::NV12),
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I420 => {
                (t == std::any::TypeId::of::<u8>()).then_some(Self::I420)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I42016 => {
                (t == std::any::TypeId::of::<u16>()).then_some(Self::I420)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I422 => {
                (t == std::any::TypeId::of::<u8>()).then_some(Self::I422)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I42216 => {
                (t == std::any::TypeId::of::<u16>()).then_some(Self::I422)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I444 => {
                (t == std::any::TypeId::of::<u8>()).then_some(Self::I444)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I44416 => {
                (t == std::any::TypeId::of::<u16>()).then_some(Self::I444)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I440 => {
                (t == std::any::TypeId::of::<u8>()).then_some(Self::I440)
            }
            vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I44016 => {
                (t == std::any::TypeId::of::<u16>()).then_some(Self::I440)
            }
            _ => None,
        }
    }
    pub(crate) fn as_vpx_image<T: YUVPixelType>(&self) -> vpx_sys::vpx_img_fmt {
        match self {
            Self::YV12 => vpx_sys::vpx_img_fmt::VPX_IMG_FMT_YV12,
            Self::NV12 => vpx_sys::vpx_img_fmt::VPX_IMG_FMT_NV12,
            Self::I420 => {
                if size_of::<T>() == 1 {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I420
                } else {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I42016
                }
            }
            Self::I422 => {
                if size_of::<T>() == 1 {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I422
                } else {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I42216
                }
            }
            Self::I444 => {
                if size_of::<T>() == 1 {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I444
                } else {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I44416
                }
            }
            Self::I440 => {
                if size_of::<T>() == 1 {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I440
                } else {
                    vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I44016
                }
            }
        }
    }
    /// Returns the buffer size in *samples* (**not bytes**) for the raw
    /// pixel data of the specified format and resolution.
    ///
    /// <div class="warning">
    ///
    /// Some formats have specific requirements for the `width` and `height`
    /// parameters:
    ///
    /// - `YV12`, `NV12` and `I420` require that both dimensions be a
    ///   multiple of 2.
    /// - `I422` only requires that `width` be a multiple of 2.
    /// - `I440` only requires that `height` be a multiple of 2.
    ///
    /// If any of these requirements is violated, this function returns
    /// an appropriate [`Error`] variant.
    ///
    /// </div>
    pub fn buffer_len(&self, width: usize, height: usize) -> Result<usize> {
        match self {
            Self::YV12 | Self::NV12 | Self::I420 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                check_dim_mult_2(height, Error::HeightNotMultipleOfTwo)?;
                // 8-bit full luma + 2x chroma at half res both W & H
                Ok(width * height + 2 * (width / 2) * (height / 2))
            }
            Self::I422 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                // 8-bit full luma + 2x chroma at half res on W & full res H
                Ok(width * height + 2 * (width / 2) * height)
            }
            Self::I440 => {
                check_dim_mult_2(height, Error::HeightNotMultipleOfTwo)?;
                // 8-bit full luma + 2x chroma at half res on W & full res H
                Ok(width * height + 2 * width * (height / 2))
            }
            // 8-bit full luma + 2x full chroma
            Self::I444 => Ok(3 * width * height),
        }
    }
    fn plane_ranges(&self, width: usize, height: usize) -> Result<PlaneRanges> {
        match self {
            // Y:V:U planar [!!!REVERSED _UV_!!!], chroma half on both W & H
            Self::YV12 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                check_dim_mult_2(height, Error::HeightNotMultipleOfTwo)?;
                let y = 0..(width * height);
                let v = y.end..(y.end + (width / 2) * (height / 2));
                let u = v.end..(v.end + (width / 2) * (height / 2));
                Ok(PlaneRanges { y, u, v })
            }
            // Y:U:V with UV interleaved, chroma half on both width and height
            Self::NV12 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                check_dim_mult_2(height, Error::HeightNotMultipleOfTwo)?;
                let y = 0..(width * height);
                let u = y.end..(y.end + 2 * (width / 2) * (height / 2));
                let v = u.clone();
                Ok(PlaneRanges { y, u, v })
            }
            // Y:U:V planar, chroma half on both width and height
            Self::I420 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                check_dim_mult_2(height, Error::HeightNotMultipleOfTwo)?;
                let y = 0..(width * height);
                let u = y.end..(y.end + (width / 2) * (height / 2));
                let v = u.end..(u.end + (width / 2) * (height / 2));
                Ok(PlaneRanges { y, u, v })
            }
            // Y:U:V planar, chroma half on width and full on height
            Self::I422 => {
                check_dim_mult_2(width, Error::WidthNotMultipleOfTwo)?;
                let y = 0..(width * height);
                let u = y.end..(y.end + (width / 2) * height);
                let v = u.end..(u.end + (width / 2) * height);
                Ok(PlaneRanges { y, u, v })
            }
            // Y:U:V planar, chroma full on width and half on height
            Self::I440 => {
                check_dim_mult_2(width, Error::HeightNotMultipleOfTwo)?;
                let y = 0..(width * height);
                let u = y.end..(y.end + width * (height / 2));
                let v = u.end..(u.end + width * (height / 2));
                Ok(PlaneRanges { y, u, v })
            }
            // Y:U:V planar, chroma full on both width and height
            Self::I444 => {
                let y = 0..(width * height);
                let u = y.end..(y.end + width * height);
                let v = u.end..(u.end + width * height);
                Ok(PlaneRanges { y, u, v })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PlaneRanges {
    y: std::ops::Range<usize>,
    u: std::ops::Range<usize>,
    v: std::ops::Range<usize>,
}

/// The low-level planar YUV image data in an image buffer. You shouldn't
/// need to manually build this struct. Instead use the
/// [`YUVImageData::from_raw_data`] method, which checks for proper plane
/// structure matching a known format.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YUVImageData<'a, T: YUVPixelType> {
    format: ImageFormat,
    width: usize,
    height: usize,
    y: &'a [T],
    y_stride: usize,
    u: &'a [T],
    u_stride: usize,
    v: &'a [T],
    v_stride: usize,
}

impl<'a, T: YUVPixelType> YUVImageData<'a, T> {
    pub(crate) fn from_vpx_image(img: &'a vpx_sys::vpx_image) -> Self {
        // This can never fail, since by the time we got here, we
        // data validated that the VPX image format makes sense in
        // `DecodedImage::data()`.
        let format = ImageFormat::from_vpx::<T>(img.fmt)
            .expect("Internal image decoding error");
        let ranges =
            format.plane_ranges(img.w as usize, img.h as usize).unwrap();
        let y = img.planes[vpx_sys::VPX_PLANE_Y as usize] as *const T;
        let y_stride = img.stride[vpx_sys::VPX_PLANE_Y as usize] as usize;
        let y_sz = ranges.y.end - ranges.y.start;
        // Flip the U and V planes when the image format is YV12.
        // Otherwise keep the normal order.
        let (u_plane, v_plane) =
            if img.fmt == vpx_sys::vpx_img_fmt::VPX_IMG_FMT_YV12 {
                (vpx_sys::VPX_PLANE_V as usize, vpx_sys::VPX_PLANE_U as usize)
            } else {
                (vpx_sys::VPX_PLANE_U as usize, vpx_sys::VPX_PLANE_V as usize)
            };
        let u = img.planes[u_plane] as *const T;
        let u_stride = img.stride[u_plane] as usize;
        let u_sz = ranges.u.end - ranges.u.start;
        let v = img.planes[v_plane] as *const T;
        let v_stride = img.stride[v_plane] as usize;
        let v_sz = ranges.v.end - ranges.v.start;
        unsafe {
            Self {
                format,
                width: img.d_w as usize,
                height: img.d_h as usize,
                y: std::slice::from_raw_parts(y, y_sz),
                y_stride,
                u: std::slice::from_raw_parts(u, u_sz),
                u_stride,
                v: std::slice::from_raw_parts(v, v_sz),
                v_stride,
            }
        }
    }
    /// Constructs a YUV image from a raw data buffer. The data buffer layout
    /// must match the provided format, width and height information.
    pub fn from_raw_data(
        format: ImageFormat,
        width: usize,
        height: usize,
        data: &'a [T],
    ) -> Result<Self> {
        let buflen_req = format.buffer_len(width, height)?;
        if data.len() != buflen_req {
            return Err(Error::ImageDataBadLength {
                expected: buflen_req,
                received: data.len(),
            });
        }
        let ranges = format.plane_ranges(width, height)?;
        Ok(Self {
            format,
            width,
            height,
            y: &data[ranges.y.clone()],
            y_stride: (ranges.y.end - ranges.y.start) / height,
            u: &data[ranges.u.clone()],
            u_stride: (ranges.u.end - ranges.u.start) / height,
            v: &data[ranges.v.clone()],
            v_stride: (ranges.v.end - ranges.v.start) / height,
        })
    }
    /// Creates a `vpx_image` struct from bytes owned by somebody *other*
    /// than the libvpx library (usually it's our Rusty library user).
    ///
    /// # Safety
    /// The created `vpx_image` doesn't own the image buffer, but refers
    /// to it, so struct mustn't be allowed to hang around. This must only
    /// be used when calling into `vpx_sys`.
    pub(crate) unsafe fn vpx_img_wrap(&self) -> Result<vpx_sys::vpx_image> {
        unsafe {
            let mut img = MaybeUninit::zeroed().assume_init();
            let result = vpx_sys::vpx_img_wrap(
                &mut img,
                self.format.as_vpx_image::<T>(),
                self.width as u32,
                self.height as u32,
                align_of::<T>() as u32,
                self.y.as_ptr() as *mut u8,
            );
            assert!(!result.is_null());
            Ok(img)
        }
    }
    /// Returns the format of the contained pixel data.
    pub fn format(&self) -> ImageFormat {
        self.format
    }
    /// Returns the display width (in pixels) of the image data.
    ///
    /// <div class="warning">
    ///
    /// Images can contain non-zero padding between successive data rows.
    /// You shouldn't use the `width` returned here to determine pixel
    /// addresses into the raw data buffers. Instead, use the strides of
    /// the respective component plane in [`YUVImagePlanes`].
    ///
    /// </div>
    pub fn width(&self) -> usize {
        self.width
    }
    /// Returns the display height (in pixels) of the image data.
    ///
    /// <div class="warning">Images can contain padding in the image planes
    /// following the actual useful image data. You shouldn't, however,
    /// display this data, or try to use it for anything useful. You should
    /// only assume the image planes to have useful data up to the number of
    /// rows returned from this method.</div>
    pub fn height(&self) -> usize {
        self.height
    }
    /// Returns the image planes, containing the raw pixel data of the image.
    pub fn planes(&self) -> YUVImagePlanes<T> {
        if self.u.as_ptr() == self.v.as_ptr() {
            assert_eq!(self.u.len(), self.v.len());
            YUVImagePlanes {
                y: self.y,
                y_stride: self.y_stride,
                uv: UVImagePlanes::Interleaved(UVImagePlanesInterleaved {
                    uv: self.u,
                    uv_stride: self.u_stride,
                }),
            }
        } else {
            YUVImagePlanes {
                y: self.y,
                y_stride: self.y_stride,
                uv: UVImagePlanes::Separate(UVImagePlanesSeparate {
                    u: self.u,
                    u_stride: self.u_stride,
                    v: self.v,
                    v_stride: self.v_stride,
                }),
            }
        }
    }
    /// Creates an owned copy of the referenced image data by copying the
    /// underlying pixel data into new buffers, which will be owned by the
    /// returned [`YUVImageDataOwned`] struct.
    pub fn to_owned(&self) -> YUVImageDataOwned<T> {
        let buflen = if matches!(self.format, ImageFormat::NV12) {
            // bi-planar data with U & V interleaved, so only use the U plane
            self.y.len() + self.u.len()
        } else {
            // normal tri-planar data
            self.y.len() + self.u.len() + self.v.len()
        };
        let mut buf = Vec::with_capacity(buflen);
        buf.extend(self.y.iter().copied());
        let y_plane = 0..self.y.len();
        let (u_plane, v_plane) = match self.format {
            ImageFormat::YV12 => {
                // U and V are reversed, so copy V first, then U
                buf.extend(self.v);
                buf.extend(self.u);
                let v_plane = y_plane.end..(y_plane.end + self.v.len());
                let u_plane = v_plane.end..(v_plane.end + self.u.len());
                (u_plane, v_plane)
            }
            ImageFormat::NV12 => {
                // format is bi-planar, so only copy the U plane contents,
                // since they also contain the V plane data.
                assert_eq!(self.u.as_ptr(), self.v.as_ptr());
                assert_eq!(self.u.len(), self.v.len());
                buf.extend(self.u);
                let u_plane = y_plane.end..(y_plane.end + self.u.len());
                let v_plane = u_plane.clone();
                (u_plane, v_plane)
            }
            _ => {
                // normal-order tri-planar data, so just copy U then V
                buf.extend(self.u);
                buf.extend(self.v);
                let u_plane = y_plane.end..(y_plane.end + self.u.len());
                let v_plane = u_plane.end..(u_plane.end + self.v.len());
                (u_plane, v_plane)
            }
        };
        YUVImageDataOwned {
            format: self.format,
            width: self.width,
            height: self.height,
            buf,
            planes: PlaneRanges {
                y: y_plane,
                u: u_plane,
                v: v_plane,
            },
            y_stride: self.y_stride,
            u_stride: self.u_stride,
            v_stride: self.v_stride,
        }
    }
}

/// An owned version of [`YUVImageData`]. To create an owned copy, call
/// [`YUVImageData::to_owned()`] to copy the pixel planes as appropriate
/// for the underlying image format.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct YUVImageDataOwned<T: YUVPixelType> {
    format: ImageFormat,
    width: usize,
    height: usize,
    buf: Vec<T>,
    planes: PlaneRanges,
    y_stride: usize,
    u_stride: usize,
    v_stride: usize,
}

impl<T: YUVPixelType> YUVImageDataOwned<T> {
    /// Creates a new image from a raw data slice containing the YUV pixel
    /// planes.
    ///
    /// - `format`: The format of the provided data. You make sure that the
    ///   underlying data follows it exactly (see [`ImageFormat`] for details).
    /// - `width`: The width of the image in pixels.
    /// - `height`: The height of the image in pixels.
    /// - `data`: The raw pixel data to be copied into the image (must be
    ///   either [`u8`] or [`u16`]). You must make sure that the data follows
    ///   the specified `format` exactly (see [`ImageFormat`] for details).
    ///   If the `data` slice is not the correct length for the `format`,
    ///   `width` and `height`, this function will return an error.
    pub fn from_raw_data(
        format: ImageFormat,
        width: usize,
        height: usize,
        data: &[T],
    ) -> Result<Self> {
        Self::from_raw_data_vec(format, width, height, data.to_vec())
    }
    /// Creates a new image from a raw data vector containing the YUV image
    /// planes.
    ///
    /// - `format`: The format of the provided data. You make sure that the
    ///   underlying data follows it exactly (see [`ImageFormat`] for details).
    /// - `width`: The width of the image in pixels.
    /// - `height`: The height of the image in pixels.
    /// - `data`: The raw pixel data to be consumed by the image (must be
    ///   either [`u8`] or [`u16`]). You must make sure that the data follows
    ///   the specified `format` exactly (see [`ImageFormat`] for details). If
    ///   the `data` vector is not the correct length for the `format`,
    ///   `width` and `height`, this function will return an error.
    pub fn from_raw_data_vec(
        format: ImageFormat,
        width: usize,
        height: usize,
        data: Vec<T>,
    ) -> Result<Self> {
        let buflen_req = format.buffer_len(width, height)?;
        if data.len() != buflen_req {
            return Err(Error::ImageDataBadLength {
                expected: buflen_req,
                received: data.len(),
            });
        }
        Ok(Self {
            format,
            width,
            height,
            buf: data,
            planes: format.plane_ranges(width, height)?,
            y_stride: 0,
            u_stride: 0,
            v_stride: 0,
        })
    }
    /// Creates a new empty YUV image (filled with zero pixels), allocating
    /// storage for the pixel data on the heap as necessary.
    /// - `format`: The format of the image.
    /// - `width`: The width of the image in pixels.
    /// - `height`: The height of the image in pixels.
    pub fn new_empty(
        format: ImageFormat,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        Self::from_raw_data_vec(
            format,
            width,
            height,
            vec![T::default(); format.buffer_len(width, height)?],
        )
    }
    /// Returns the format of the contained pixel data.
    pub fn format(&self) -> ImageFormat {
        self.format
    }
    /// Returns the display width (in pixels) of the image data.
    ///
    /// <div class="warning">
    ///
    /// Images can contain non-zero padding between successive data rows.
    /// You shouldn't use the `width` returned here to determine pixel
    /// addresses into the raw data buffers. Instead, use the strides of
    /// the respective component plane in [`YUVImagePlanes`] or
    /// [`YUVImagePlanesMut`].
    ///
    /// </div>
    pub fn width(&self) -> usize {
        self.width
    }
    /// Returns the display height (in pixels) of the image data.
    ///
    /// <div class="warning">Images can contain padding in the image planes
    /// following the actual useful image data. You shouldn't, however,
    /// display this data, or try to use it for anything useful. You should
    /// only assume the image planes to have useful data up to the number of
    /// rows returned from this method.</div>
    pub fn height(&self) -> usize {
        self.height
    }
    /// Returns the image planes, containing the raw pixel data of the image.
    pub fn planes(&self) -> YUVImagePlanes<T> {
        if self.planes.u == self.planes.v {
            YUVImagePlanes {
                y: &self.buf[self.planes.y.clone()],
                y_stride: self.y_stride,
                uv: UVImagePlanes::Interleaved(UVImagePlanesInterleaved {
                    uv: &self.buf[self.planes.u.clone()],
                    uv_stride: 2 * self.u_stride,
                }),
            }
        } else {
            YUVImagePlanes {
                y: &self.buf[self.planes.y.clone()],
                y_stride: self.y_stride,
                uv: UVImagePlanes::Separate(UVImagePlanesSeparate {
                    u: &self.buf[self.planes.u.clone()],
                    u_stride: self.u_stride,
                    v: &self.buf[self.planes.v.clone()],
                    v_stride: self.v_stride,
                }),
            }
        }
    }
    /// Returns the image planes in mutable form, to allow editing the pixel
    /// data in each plane.
    pub fn planes_mut(&mut self) -> YUVImagePlanesMut<T> {
        let (y, uv) = self.buf.split_at_mut(self.planes.y.end);
        if self.planes.u == self.planes.v {
            // bi-planar data with UV interleaved, so only produce two planes
            YUVImagePlanesMut {
                y,
                y_stride: self.y_stride,
                uv: UVImagePlanesMut::Interleaved(
                    UVImagePlanesInterleavedMut {
                        uv,
                        uv_stride: 2 * self.u_stride,
                    },
                ),
            }
        } else {
            let (u, v) = if self.planes.u.start < self.planes.v.start {
                // Normally ordered U and V planes, split after U
                uv.split_at_mut(self.planes.u.end - self.planes.u.start)
            } else {
                // U and V planes are reversed
                let (v, u) =
                    uv.split_at_mut(self.planes.v.end - self.planes.v.start);
                // flip them back around
                (u, v)
            };
            YUVImagePlanesMut {
                y,
                y_stride: self.y_stride,
                uv: UVImagePlanesMut::Separate(UVImagePlanesSeparateMut {
                    u,
                    u_stride: self.u_stride,
                    v,
                    v_stride: self.v_stride,
                }),
            }
        }
    }
    /// Returns low-level access to the contiguous memory buffer holding
    /// the image planes. This pixel data is laid out exactly as described
    /// in [`ImageFormat`], including any stride padding. You should only
    /// use this if you have external libraries which require that you pass
    /// contiguous YUV image buffers to them, rather than individual planes.
    pub fn as_raw_data(&self) -> &[T] {
        &self.buf
    }
    /// Same as [`YUVImageDataOwned::as_raw_data()`], but the returned
    /// reference is exclusive, to allow mutating the data.
    pub fn as_raw_data_mut(&mut self) -> &mut [T] {
        &mut self.buf
    }
}

/// This struct holds slices to YUV image planes in a neat structure.
/// Please note that even if the source image format is `YV12` with the
/// `V` and `U` planes reversed, they will **NOT** be reversed in this
/// struct. That means you **DON'T** need to flip the order of
/// manipulations in your code. You simply use the planes exactly as
/// they are named.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct YUVImagePlanes<'a, T: YUVPixelType> {
    /// The raw pixel data for the luma (Y) plane.
    pub y: &'a [T],
    y_stride: usize,
    /// The U and V planes, either separate or interleaved (depending on
    /// the image format).
    pub uv: UVImagePlanes<'a, T>,
}

impl<T: YUVPixelType> YUVImagePlanes<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the luma plane.
    #[inline]
    pub fn y_stride(&self) -> usize {
        self.y_stride
    }
}

/// Holds mutable references to YUV image planes and their pixel data.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct YUVImagePlanesMut<'a, T: YUVPixelType> {
    /// The raw pixel data for the luma (Y) plane.
    pub y: &'a mut [T],
    y_stride: usize,
    /// The U and V planes, either separate or interleaved (depending on
    /// the image format).
    pub uv: UVImagePlanesMut<'a, T>,
}

impl<T: YUVPixelType> YUVImagePlanesMut<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the luma plane.
    #[inline]
    pub fn y_stride(&self) -> usize {
        self.y_stride
    }
}

/// Holds the U and V chroma planes. Depending on the image format,
/// these are either stored separate, or interleaved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UVImagePlanes<'a, T: YUVPixelType> {
    /// The image format uses separate planes for the U and V chroma planes.
    Separate(UVImagePlanesSeparate<'a, T>),
    /// The image format interleaves the U and V chroma data in a single plane.
    Interleaved(UVImagePlanesInterleaved<'a, T>),
}

/// Holds the separate U and V chroma planes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UVImagePlanesSeparate<'a, T: YUVPixelType> {
    /// The raw pixel data for the chroma U plane.
    pub u: &'a [T],
    u_stride: usize,
    /// The raw pixel data for the chroma V plane.
    pub v: &'a [T],
    v_stride: usize,
}

impl<T: YUVPixelType> UVImagePlanesSeparate<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the chroma U plane.
    #[inline]
    pub fn u_stride(&self) -> usize {
        self.u_stride
    }
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the chroma V plane.
    #[inline]
    pub fn v_stride(&self) -> usize {
        self.v_stride
    }
}

/// Holds the interleaved U and V chroma planes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UVImagePlanesInterleaved<'a, T: PartialEq + Eq> {
    /// The raw pixel data for the interleaved chroma UV plane. The samples
    /// are interleaved in exactly this order: the first is a U sample,
    /// followed by a V sample, followed again by a U sample for the next
    /// pixel to the right, etc.
    pub uv: &'a [T],
    uv_stride: usize,
}

impl<T: YUVPixelType> UVImagePlanesInterleaved<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the interleaved chroma UV plane.
    ///
    /// <div class="warning">You shouldn't double this number in order to
    /// account for data interleaving. This number is the **EXACT** number
    /// of samples you need to skip to get to the next row of interleaved
    /// samples.</div>
    #[inline]
    pub fn uv_stride(&self) -> usize {
        self.uv_stride
    }
}

/// Holds the mutable U and V chroma planes. Depending on the image format,
/// these are either stored separate, or interleaved.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum UVImagePlanesMut<'a, T: YUVPixelType> {
    /// The image format uses separate planes for the U and V chroma planes.
    Separate(UVImagePlanesSeparateMut<'a, T>),
    /// The image format interleaves the U and V chroma data in a single plane.
    Interleaved(UVImagePlanesInterleavedMut<'a, T>),
}

/// Holds the separate mutable U and V chroma planes.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct UVImagePlanesSeparateMut<'a, T: YUVPixelType> {
    /// The raw mutable pixel data for the chroma U plane.
    pub u: &'a mut [T],
    u_stride: usize,
    /// The raw mutable pixel data for the chroma V plane.
    pub v: &'a mut [T],
    v_stride: usize,
}

impl<T: YUVPixelType> UVImagePlanesSeparateMut<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the chroma U plane.
    #[inline]
    pub fn u_stride(&self) -> usize {
        self.u_stride
    }
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the chroma V plane.
    #[inline]
    pub fn v_stride(&self) -> usize {
        self.v_stride
    }
}

/// Holds the interleaved mutable U and V chroma planes.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct UVImagePlanesInterleavedMut<'a, T: YUVPixelType> {
    /// The raw mutable pixel data for the interleaved chroma UV plane. The
    /// samples are interleaved in exactly this order: the first is a U sample,
    /// followed by a V sample, followed again by a U sample for the next
    /// pixel to the right, etc.
    pub uv: &'a mut [T],
    uv_stride: usize,
}

impl<T: YUVPixelType> UVImagePlanesInterleavedMut<'_, T> {
    /// Returns the stride (in samples, **NOT** bytes) between successive
    /// rows in the interleaved chroma UV plane.
    ///
    /// <div class="warning">You shouldn't double this number in order to
    /// account for data interleaving. This number is the **EXACT** number
    /// of samples you need to skip to get to the next row of interleaved
    /// samples.</div>
    #[inline]
    pub fn uv_stride(&self) -> usize {
        self.uv_stride
    }
}

#[cfg(test)]
mod test {
    use super::{ImageFormat, ImageFormat::*, YUVImageData};
    use super::{UVImagePlanes, UVImagePlanesMut, YUVImagePlanes};
    use crate::image::{YUVImagePlanesMut, YUVPixelType};
    use crate::{Result, YUVImageDataOwned};

    const WIDTH: usize = 4;
    const HEIGHT: usize = 4;
    // The test pixel values here are laid out in a very specific pattern.
    // Y pixels have values 0-15
    // U pixels have values 16-31 (if the plane isn't that large, we stop short)
    // V pixels have values 32-47
    const Y_PIXEL_START: usize = 0;
    const U_PIXEL_START: usize = 16;
    const V_PIXEL_START: usize = 32;
    const DATA_I420: [u8; 24] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U
        16, 17, 18, 19, //
        // Chroma V
        32, 33, 34, 35,
    ];
    const DATA_YV12_REV: [u8; 24] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma V
        32, 33, 34, 35, //
        // Chroma U
        16, 17, 18, 19,
    ];
    const DATA_NV12: [u8; 24] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U/V interleaved
        16, 32, 17, 33, 18, 34, 19, 35,
    ];
    const DATA_I420_16: [u16; 24] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U/V
        16, 17, 18, 19, //
        // Chroma U/V
        32, 33, 34, 35,
    ];
    const DATA_I422: [u8; 32] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U
        16, 17, 18, 19, 20, 21, 22, 23, //
        // Chroma V
        32, 33, 34, 35, 36, 37, 38, 39,
    ];
    const DATA_I422_16: [u16; 32] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U
        16, 17, 18, 19, 20, 21, 22, 23, //
        // Chroma V
        32, 33, 34, 35, 36, 37, 38, 39,
    ];
    const DATA_I444: [u8; 48] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, //
        // Chroma V
        32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    ];
    const DATA_I444_16: [u16; 48] = [
        // Luma
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        // Chroma U
        16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, //
        // Chroma V
        32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    ];

    #[test]
    fn test_image_format_buffer_len() -> Result<()> {
        // Text buffer length estimations
        assert_eq!(ImageFormat::YV12.buffer_len(4, 4)?, DATA_I420.len());
        assert_eq!(ImageFormat::NV12.buffer_len(4, 4)?, DATA_I420.len());
        assert_eq!(I420.buffer_len(4, 4)?, DATA_I420.len());
        assert_eq!(I422.buffer_len(4, 4)?, DATA_I422.len());
        assert_eq!(I440.buffer_len(4, 4)?, DATA_I422.len());
        assert_eq!(I444.buffer_len(4, 4)?, DATA_I444.len());
        Ok(())
    }
    #[test]
    fn test_image_format_sizes() -> Result<()> {
        // Both width and height must be even.
        assert!(YV12.buffer_len(1, 2).is_err());
        assert!(YV12.buffer_len(2, 1).is_err());
        assert!(YV12.buffer_len(2, 2).is_ok());
        assert!(NV12.buffer_len(1, 2).is_err());
        assert!(NV12.buffer_len(2, 1).is_err());
        assert!(NV12.buffer_len(2, 2).is_ok());
        assert!(I420.buffer_len(1, 2).is_err());
        assert!(I420.buffer_len(2, 1).is_err());
        assert!(I420.buffer_len(2, 2).is_ok());
        // Width must be even.
        assert!(I422.buffer_len(1, 2).is_err());
        assert!(I422.buffer_len(2, 1).is_ok());
        // Height must be even.
        assert!(I440.buffer_len(1, 2).is_ok());
        assert!(I440.buffer_len(2, 1).is_err());
        // Both width and height can be odd.
        assert!(I444.buffer_len(1, 2).is_ok());
        assert!(I444.buffer_len(2, 1).is_ok());
        Ok(())
    }
    #[test]
    fn test_imagedata_metainfo() -> Result<()> {
        let img = YUVImageData::from_raw_data(YV12, WIDTH, HEIGHT, &DATA_I420)?;
        assert_eq!(img.format(), YV12);
        assert_eq!(img.width(), WIDTH);
        assert_eq!(img.height(), HEIGHT);
        let img = YUVImageData::from_raw_data(
            ImageFormat::I420,
            WIDTH,
            HEIGHT,
            &DATA_I420_16,
        )?;
        assert_eq!(img.format(), I420);
        assert_eq!(img.width(), WIDTH);
        assert_eq!(img.height(), HEIGHT);
        Ok(())
    }
    #[test]
    fn test_imagedata_planes() -> Result<()> {
        fn test_img<T: YUVPixelType + Into<usize>>(
            format: ImageFormat,
            width: usize,
            height: usize,
            separate_planes: bool,
            data: &[T],
            chroma_sub_fact: usize,
        ) -> Result<()> {
            let img = YUVImageData::from_raw_data(format, width, height, data)?;
            test_planes_impl(
                width,
                height,
                &img.planes(),
                separate_planes,
                chroma_sub_fact,
            )
        }
        fn test_img_owned<T: YUVPixelType + Into<usize>>(
            format: ImageFormat,
            width: usize,
            height: usize,
            separate_planes: bool,
            data: &[T],
            chroma_sub_fact: usize,
        ) -> Result<()> {
            let mut img =
                YUVImageDataOwned::from_raw_data(format, width, height, data)?;
            test_planes_impl(
                width,
                height,
                &img.planes(),
                separate_planes,
                chroma_sub_fact,
            )?;
            test_planes_mut_impl(
                width,
                height,
                &mut img.planes_mut(),
                separate_planes,
                chroma_sub_fact,
            )
        }
        fn check_plane_pixels<T: YUVPixelType + Into<usize>>(
            plane: &[T],
            start_off: usize,
        ) {
            plane.iter().enumerate().for_each(|(idx, pixel)| {
                assert_eq!(idx + start_off, (*pixel).into());
            });
        }
        fn test_planes_impl<T: YUVPixelType + Into<usize>>(
            width: usize,
            height: usize,
            planes: &YUVImagePlanes<T>,
            separate_planes: bool,
            chroma_sub_fact: usize,
        ) -> Result<()> {
            assert_eq!(planes.y.len(), width * height);
            check_plane_pixels(planes.y, Y_PIXEL_START);
            match &planes.uv {
                UVImagePlanes::Separate(uv_planes) => {
                    assert!(separate_planes);
                    let req_len = width * height / chroma_sub_fact;
                    assert_eq!(uv_planes.u.len(), req_len);
                    check_plane_pixels(uv_planes.u, U_PIXEL_START);
                    assert_eq!(uv_planes.v.len(), req_len);
                    check_plane_pixels(uv_planes.v, V_PIXEL_START);
                }
                UVImagePlanes::Interleaved(uv_plane) => {
                    assert!(!separate_planes);
                    let req_len = 2 * width * height / chroma_sub_fact;
                    assert_eq!(uv_plane.uv.len(), req_len);
                    uv_plane.uv.chunks(2).enumerate().for_each(
                        |(idx, pixels)| {
                            let [u_pixel, v_pixel] = *pixels else {
                                unreachable!();
                            };
                            assert_eq!(idx + U_PIXEL_START, u_pixel.into());
                            assert_eq!(idx + V_PIXEL_START, v_pixel.into());
                        },
                    );
                }
            };
            Ok(())
        }
        fn test_planes_mut_impl<T: YUVPixelType + Into<usize>>(
            width: usize,
            height: usize,
            planes: &mut YUVImagePlanesMut<T>,
            separate_planes: bool,
            chroma_sub_fact: usize,
        ) -> Result<()> {
            assert_eq!(planes.y.len(), width * height);
            check_plane_pixels(planes.y, Y_PIXEL_START);
            match &planes.uv {
                UVImagePlanesMut::Separate(uv_planes) => {
                    assert!(separate_planes);
                    let req_len = width * height / chroma_sub_fact;
                    assert_eq!(uv_planes.u.len(), req_len);
                    check_plane_pixels(uv_planes.u, U_PIXEL_START);
                    assert_eq!(uv_planes.v.len(), req_len);
                    check_plane_pixels(uv_planes.v, V_PIXEL_START);
                }
                UVImagePlanesMut::Interleaved(uv_plane) => {
                    assert!(!separate_planes);
                    let req_len = 2 * width * height / chroma_sub_fact;
                    assert_eq!(uv_plane.uv.len(), req_len);
                    uv_plane.uv.chunks(2).enumerate().for_each(
                        |(idx, pixels)| {
                            let [u_pixel, v_pixel] = *pixels else {
                                unreachable!();
                            };
                            assert_eq!(idx + U_PIXEL_START, u_pixel.into());
                            assert_eq!(idx + V_PIXEL_START, v_pixel.into());
                        },
                    );
                }
            };
            Ok(())
        }
        test_img(YV12, WIDTH, HEIGHT, true, &DATA_YV12_REV, 4)?;
        test_img(NV12, WIDTH, HEIGHT, false, &DATA_NV12, 4)?;
        test_img(I420, WIDTH, HEIGHT, true, &DATA_I420, 4)?;
        test_img(I422, WIDTH, HEIGHT, true, &DATA_I422, 2)?;
        test_img(I440, WIDTH, HEIGHT, true, &DATA_I422, 2)?;
        test_img(I444, WIDTH, HEIGHT, true, &DATA_I444, 1)?;
        test_img(I420, WIDTH, HEIGHT, true, &DATA_I420_16, 4)?;
        test_img(I422, WIDTH, HEIGHT, true, &DATA_I422_16, 2)?;
        test_img(I440, WIDTH, HEIGHT, true, &DATA_I422_16, 2)?;
        test_img(I444, WIDTH, HEIGHT, true, &DATA_I444_16, 1)?;

        test_img_owned(YV12, WIDTH, HEIGHT, true, &DATA_YV12_REV, 4)?;
        test_img_owned(NV12, WIDTH, HEIGHT, false, &DATA_NV12, 4)?;
        test_img_owned(I420, WIDTH, HEIGHT, true, &DATA_I420, 4)?;
        test_img_owned(I422, WIDTH, HEIGHT, true, &DATA_I422, 2)?;
        test_img_owned(I440, WIDTH, HEIGHT, true, &DATA_I422, 2)?;
        test_img_owned(I444, WIDTH, HEIGHT, true, &DATA_I444, 1)?;
        test_img_owned(I420, WIDTH, HEIGHT, true, &DATA_I420_16, 4)?;
        test_img_owned(I422, WIDTH, HEIGHT, true, &DATA_I422_16, 2)?;
        test_img_owned(I440, WIDTH, HEIGHT, true, &DATA_I422_16, 2)?;
        test_img_owned(I444, WIDTH, HEIGHT, true, &DATA_I444_16, 1)?;
        Ok(())
    }
    #[test]
    fn test_send() {
        fn assert_send<T: Send>() {}
        assert_send::<YUVImageData<u8>>();
        assert_send::<YUVImageData<u16>>();
        assert_send::<YUVImageDataOwned<u8>>();
        assert_send::<YUVImageDataOwned<u16>>();
    }
    #[test]
    fn test_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<YUVImageData<u8>>();
        assert_sync::<YUVImageData<u16>>();
        assert_sync::<YUVImageDataOwned<u8>>();
        assert_sync::<YUVImageDataOwned<u16>>();
    }
}
