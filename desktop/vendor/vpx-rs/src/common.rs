/*
 * Copyright (c) 2025, Saso Kiselkov. All rights reserved.
 *
 * Use of this source code is governed by the 2-clause BSD license,
 * which can be found in a file named LICENSE, located at the root
 * of this source tree.
 */

/// Stream information retrieved from either an `Encoder` or `Decoder` instance.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub struct StreamInfo {
    /// The stream's display width in pixels.
    pub width: u32,
    /// The stream's display height in pixels.
    pub height: u32,
    /// True if the returned decoded frame will be a keyframe.
    pub is_keyframe: bool,
}

impl From<vpx_sys::vpx_codec_stream_info> for StreamInfo {
    fn from(value: vpx_sys::vpx_codec_stream_info) -> Self {
        Self {
            width: value.w,
            height: value.h,
            is_keyframe: value.is_kf != 0,
        }
    }
}

pub(crate) fn get_stream_info(
    ctx: &mut vpx_sys::vpx_codec_ctx,
) -> Option<StreamInfo> {
    let mut si = vpx_sys::vpx_codec_stream_info {
        sz: size_of::<vpx_sys::vpx_codec_stream_info>() as _,
        w: 0,
        h: 0,
        is_kf: 0,
    };
    let error = unsafe { vpx_sys::vpx_codec_get_stream_info(ctx, &mut si) };
    (error == vpx_sys::VPX_CODEC_OK).then(|| si.into())
}
