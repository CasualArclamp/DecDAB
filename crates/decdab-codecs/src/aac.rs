// made by CasualArclamp
//! The DAB+ audio decoder: FDK-AAC fed raw access units (`TT_MP4_RAW`)
//! after an AudioSpecificConfig built from the superframe header. Follows
//! DecDRM's FDK wrapper (decdrm-codecs/src/fdk.rs).

use std::ptr::NonNull;

use decdab_fdk_sys as ffi;

use crate::{CodecError, Pcm};

/// Output buffer: 2048 samples × 2 channels covers HE-AAC's 1920 a frame.
const OUT_CAPACITY: usize = 8192;

pub struct AacDecoder {
    handle: NonNull<ffi::AAC_DECODER_INSTANCE>,
    asc: Vec<u8>,
    input: Vec<u8>,
    out: Vec<i16>,
}

// SAFETY: the instance is owned exclusively by this value and FDK keeps no
// thread-affine state, so moving it to another thread is sound (as in
// DecDRM).
unsafe impl Send for AacDecoder {}

impl Drop for AacDecoder {
    fn drop(&mut self) {
        // SAFETY: opened by aacDecoder_Open, closed exactly once.
        unsafe { ffi::aacDecoder_Close(self.handle.as_ptr()) }
    }
}

fn err(code: i32, what: &'static str) -> CodecError {
    CodecError::Fdk(code, what)
}

impl AacDecoder {
    /// A decoder for the stream an AudioSpecificConfig describes.
    pub fn new(asc: &[u8]) -> Result<Self, CodecError> {
        // SAFETY: a plain constructor; NULL is handled.
        let raw = unsafe { ffi::aacDecoder_Open(ffi::TT_MP4_RAW, 1) };
        let handle = NonNull::new(raw).ok_or(err(0, "open"))?;
        let d = AacDecoder {
            handle,
            asc: asc.to_vec(),
            input: Vec::with_capacity(2048),
            out: vec![0; OUT_CAPACITY],
        };
        let mut conf = d.asc.clone();
        let mut p = conf.as_mut_ptr();
        let len = conf.len() as u32;
        // SAFETY: p/len describe the live `conf`; FDK copies it.
        let e = unsafe { ffi::aacDecoder_ConfigRaw(d.handle.as_ptr(), &mut p, &len) };
        if e != ffi::AAC_DEC_OK {
            return Err(err(e, "configure"));
        }
        // SAFETY: valid handle, integer parameter.
        unsafe {
            ffi::aacDecoder_SetParam(d.handle.as_ptr(), ffi::AAC_PCM_MAX_OUTPUT_CHANNELS, 2);
            // Mono out as two channels, so the player always sees stereo.
            ffi::aacDecoder_SetParam(d.handle.as_ptr(), ffi::AAC_PCM_MIN_OUTPUT_CHANNELS, 2);
        }
        Ok(d)
    }

    /// The configuration it was opened with.
    pub fn asc(&self) -> &[u8] {
        &self.asc
    }

    /// Decode one access unit; `None` (after a failed CRC) conceals.
    pub fn decode(&mut self, au: Option<&[u8]>) -> Result<Pcm, CodecError> {
        let flags = match au {
            Some(au) if !au.is_empty() => {
                self.input.clear();
                self.input.extend_from_slice(au);
                let mut p = self.input.as_mut_ptr();
                let size = self.input.len() as u32;
                let mut valid = size;
                // SAFETY: p/size describe the live input; FDK copies and
                // updates `valid`.
                let e = unsafe {
                    ffi::aacDecoder_Fill(self.handle.as_ptr(), &mut p, &size, &mut valid)
                };
                if e != ffi::AAC_DEC_OK {
                    return Err(err(e, "fill"));
                }
                0
            }
            _ => ffi::AACDEC_CONCEAL,
        };
        // SAFETY: `out` holds OUT_CAPACITY samples, the size passed.
        let e = unsafe {
            ffi::aacDecoder_DecodeFrame(
                self.handle.as_ptr(),
                self.out.as_mut_ptr(),
                OUT_CAPACITY as i32,
                flags,
            )
        };
        if !ffi::IS_OUTPUT_VALID(e) {
            // SAFETY: valid handle; flushes the transport buffer.
            unsafe {
                ffi::aacDecoder_SetParam(self.handle.as_ptr(), ffi::AAC_TPDEC_CLEAR_BUFFER, 1)
            };
            return Err(err(e, "decode"));
        }
        // SAFETY: the pointer is into the live instance; copied at once.
        let si = unsafe { ffi::aacDecoder_GetStreamInfo(self.handle.as_ptr()).as_ref() }
            .copied()
            .ok_or(err(0, "stream info"))?;
        let (ch, n) = (si.numChannels.max(0) as usize, si.frameSize.max(0) as usize);
        if ch == 0 || ch * n > OUT_CAPACITY {
            return Err(err(0, "no output"));
        }
        Ok(Pcm {
            rate: si.sampleRate.max(0) as u32,
            channels: ch as u16,
            samples: self.out[..ch * n].to_vec(),
            concealed: e != ffi::AAC_DEC_OK || flags != 0,
        })
    }
}
