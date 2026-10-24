/* DecDAB helper shim compiled together with FDK-AAC.
 *
 * 1. Exports sizeof/offsetof of every struct (and enum size) that
 *    decdab-fdk-sys transcribes by hand, so Rust unit tests can check the
 *    hand-written #[repr(C)] layouts against the real compiler's view.
 * 2. Exports the AAC Huffman code tables of the FDK *encoder* (spectral
 *    codebooks 1..11 and the scalefactor codebook) in the canonical ISO/IEC
 *    14496-3 index order. decdab-codecs uses them to re-pack FDK's
 *    MPEG-4 (GA) access units into DRM (ER, VCB11 + HCR) access units without
 *    re-typing ~1400 table entries by hand.
 *
 * This file is part of DecDAB (GPL-3.0-or-later; from DecDRM); it only reads FDK data.
 */

#include <stddef.h>

#include "aacdecoder_lib.h"
#include "aacenc_lib.h"
#include "aacEnc_rom.h" /* libAACenc/src: FDKaacEnc_huff_* tables */

typedef struct {
  const char *name;
  size_t value;
} DecdrmLayoutEntry;

#define DECDAB_SIZE(T) {"sizeof " #T, sizeof(T)}
#define DECDAB_ALIGN(T) {"alignof " #T, alignof(T)}
#define DECDAB_OFF(T, f) {#T "." #f, offsetof(T, f)}

static const DecdrmLayoutEntry kLayout[] = {
    /* scalar typedefs and enums */
    DECDAB_SIZE(INT),
    DECDAB_SIZE(UINT),
    DECDAB_SIZE(LONG),
    DECDAB_SIZE(INT64),
    DECDAB_SIZE(INT_PCM),
    DECDAB_SIZE(AUDIO_OBJECT_TYPE),
    DECDAB_SIZE(TRANSPORT_TYPE),
    DECDAB_SIZE(CHANNEL_MODE),
    DECDAB_SIZE(AUDIO_CHANNEL_TYPE),
    DECDAB_SIZE(FDK_MODULE_ID),
    DECDAB_SIZE(AAC_DECODER_ERROR),
    DECDAB_SIZE(AACDEC_PARAM),
    DECDAB_SIZE(AACENC_ERROR),
    DECDAB_SIZE(AACENC_PARAM),
    DECDAB_SIZE(AACENC_BufferIdentifier),

    /* CStreamInfo */
    DECDAB_SIZE(CStreamInfo),
    DECDAB_ALIGN(CStreamInfo),
    DECDAB_OFF(CStreamInfo, sampleRate),
    DECDAB_OFF(CStreamInfo, frameSize),
    DECDAB_OFF(CStreamInfo, numChannels),
    DECDAB_OFF(CStreamInfo, pChannelType),
    DECDAB_OFF(CStreamInfo, pChannelIndices),
    DECDAB_OFF(CStreamInfo, aacSampleRate),
    DECDAB_OFF(CStreamInfo, profile),
    DECDAB_OFF(CStreamInfo, aot),
    DECDAB_OFF(CStreamInfo, channelConfig),
    DECDAB_OFF(CStreamInfo, bitRate),
    DECDAB_OFF(CStreamInfo, aacSamplesPerFrame),
    DECDAB_OFF(CStreamInfo, aacNumChannels),
    DECDAB_OFF(CStreamInfo, extAot),
    DECDAB_OFF(CStreamInfo, extSamplingRate),
    DECDAB_OFF(CStreamInfo, outputDelay),
    DECDAB_OFF(CStreamInfo, flags),
    DECDAB_OFF(CStreamInfo, epConfig),
    DECDAB_OFF(CStreamInfo, numLostAccessUnits),
    DECDAB_OFF(CStreamInfo, numTotalBytes),
    DECDAB_OFF(CStreamInfo, numBadBytes),
    DECDAB_OFF(CStreamInfo, numTotalAccessUnits),
    DECDAB_OFF(CStreamInfo, numBadAccessUnits),
    DECDAB_OFF(CStreamInfo, drcProgRefLev),
    DECDAB_OFF(CStreamInfo, drcPresMode),
    DECDAB_OFF(CStreamInfo, outputLoudness),

    /* LIB_INFO */
    DECDAB_SIZE(LIB_INFO),
    DECDAB_ALIGN(LIB_INFO),
    DECDAB_OFF(LIB_INFO, title),
    DECDAB_OFF(LIB_INFO, build_date),
    DECDAB_OFF(LIB_INFO, build_time),
    DECDAB_OFF(LIB_INFO, module_id),
    DECDAB_OFF(LIB_INFO, version),
    DECDAB_OFF(LIB_INFO, flags),
    DECDAB_OFF(LIB_INFO, versionStr),

    /* AACENC_InfoStruct */
    DECDAB_SIZE(AACENC_InfoStruct),
    DECDAB_ALIGN(AACENC_InfoStruct),
    DECDAB_OFF(AACENC_InfoStruct, maxOutBufBytes),
    DECDAB_OFF(AACENC_InfoStruct, maxAncBytes),
    DECDAB_OFF(AACENC_InfoStruct, inBufFillLevel),
    DECDAB_OFF(AACENC_InfoStruct, inputChannels),
    DECDAB_OFF(AACENC_InfoStruct, frameLength),
    DECDAB_OFF(AACENC_InfoStruct, nDelay),
    DECDAB_OFF(AACENC_InfoStruct, nDelayCore),
    DECDAB_OFF(AACENC_InfoStruct, confBuf),
    DECDAB_OFF(AACENC_InfoStruct, confSize),

    /* AACENC_BufDesc */
    DECDAB_SIZE(AACENC_BufDesc),
    DECDAB_ALIGN(AACENC_BufDesc),
    DECDAB_OFF(AACENC_BufDesc, numBufs),
    DECDAB_OFF(AACENC_BufDesc, bufs),
    DECDAB_OFF(AACENC_BufDesc, bufferIdentifiers),
    DECDAB_OFF(AACENC_BufDesc, bufSizes),
    DECDAB_OFF(AACENC_BufDesc, bufElSizes),

    /* AACENC_InArgs / AACENC_OutArgs */
    DECDAB_SIZE(AACENC_InArgs),
    DECDAB_ALIGN(AACENC_InArgs),
    DECDAB_OFF(AACENC_InArgs, numInSamples),
    DECDAB_OFF(AACENC_InArgs, numAncBytes),
    DECDAB_SIZE(AACENC_OutArgs),
    DECDAB_ALIGN(AACENC_OutArgs),
    DECDAB_OFF(AACENC_OutArgs, numOutBytes),
    DECDAB_OFF(AACENC_OutArgs, numInSamples),
    DECDAB_OFF(AACENC_OutArgs, numAncBytes),
    DECDAB_OFF(AACENC_OutArgs, bitResState),
};

extern "C" {

/* Number of entries in the layout table. */
size_t decdab_fdk_layout_count(void) {
  return sizeof(kLayout) / sizeof(kLayout[0]);
}

/* Name ("sizeof X", "alignof X" or "Struct.field") and value of entry `i`.
 * Returns NULL (and leaves *value untouched) when `i` is out of range. */
const char *decdab_fdk_layout_entry(size_t i, size_t *value) {
  if (i >= decdab_fdk_layout_count()) return NULL;
  if (value != NULL) *value = kLayout[i].value;
  return kLayout[i].name;
}

/* Copies spectral Huffman codebook `cb` (1..11) into `codes`/`lens` in the
 * canonical ISO/IEC 14496-3 codebook index order:
 *   books 1,2  : idx = 27(w+1) + 9(x+1) + 3(y+1) + (z+1), values -1..1
 *   books 3,4  : idx = 27|w| + 9|x| + 3|y| + |z|,         values  0..2
 *   books 5,6  : idx = 9(y+4) + (z+4),                     values -4..4
 *   books 7,8  : idx = 8|y| + |z|,                         values  0..7
 *   books 9,10 : idx = 13|y| + |z|,                        values  0..12
 *   book 11    : idx = 17|y| + |z|,                        values  0..16
 * Sign bits and escape sequences are NOT part of these codewords.
 * Returns the number of entries written, or -1 on bad arguments. */
int decdab_fdk_huffman_spectral(int cb, unsigned short *codes,
                                unsigned char *lens, int capacity) {
  int n = 0;
  if (codes == NULL || lens == NULL) return -1;
  switch (cb) {
    case 1:
    case 2:
    case 3:
    case 4: {
      const USHORT *ctab = (cb == 1)   ? &FDKaacEnc_huff_ctab1[0][0][0][0]
                           : (cb == 2) ? &FDKaacEnc_huff_ctab2[0][0][0][0]
                           : (cb == 3) ? &FDKaacEnc_huff_ctab3[0][0][0][0]
                                       : &FDKaacEnc_huff_ctab4[0][0][0][0];
      const ULONG *ltab = (cb <= 2) ? &FDKaacEnc_huff_ltab1_2[0][0][0][0]
                                    : &FDKaacEnc_huff_ltab3_4[0][0][0][0];
      const int hi = (cb == 1 || cb == 3);
      n = 81;
      if (capacity < n) return -1;
      for (int i = 0; i < n; i++) {
        codes[i] = ctab[i];
        lens[i] = (unsigned char)(hi ? (ltab[i] >> 16) : (ltab[i] & 0xffff));
      }
      return n;
    }
    case 5:
    case 6: {
      const USHORT *ctab = (cb == 5) ? &FDKaacEnc_huff_ctab5[0][0]
                                     : &FDKaacEnc_huff_ctab6[0][0];
      const ULONG *ltab = &FDKaacEnc_huff_ltab5_6[0][0];
      n = 81;
      if (capacity < n) return -1;
      for (int i = 0; i < n; i++) {
        codes[i] = ctab[i];
        lens[i] = (unsigned char)((cb == 5) ? (ltab[i] >> 16)
                                            : (ltab[i] & 0xffff));
      }
      return n;
    }
    case 7:
    case 8: {
      const USHORT *ctab = (cb == 7) ? &FDKaacEnc_huff_ctab7[0][0]
                                     : &FDKaacEnc_huff_ctab8[0][0];
      const ULONG *ltab = &FDKaacEnc_huff_ltab7_8[0][0];
      n = 64;
      if (capacity < n) return -1;
      for (int i = 0; i < n; i++) {
        codes[i] = ctab[i];
        lens[i] = (unsigned char)((cb == 7) ? (ltab[i] >> 16)
                                            : (ltab[i] & 0xffff));
      }
      return n;
    }
    case 9:
    case 10: {
      const USHORT *ctab = (cb == 9) ? &FDKaacEnc_huff_ctab9[0][0]
                                     : &FDKaacEnc_huff_ctab10[0][0];
      const ULONG *ltab = &FDKaacEnc_huff_ltab9_10[0][0];
      n = 169;
      if (capacity < n) return -1;
      for (int i = 0; i < n; i++) {
        codes[i] = ctab[i];
        lens[i] = (unsigned char)((cb == 9) ? (ltab[i] >> 16)
                                            : (ltab[i] & 0xffff));
      }
      return n;
    }
    case 11: {
      n = 289;
      if (capacity < n) return -1;
      for (int y = 0; y < 17; y++) {
        for (int z = 0; z < 17; z++) {
          codes[17 * y + z] = FDKaacEnc_huff_ctab11[y][z];
          lens[17 * y + z] = FDKaacEnc_huff_ltab11[y][z];
        }
      }
      return n;
    }
    default:
      return -1;
  }
}

/* Copies the scalefactor codebook (121 entries, index = delta + 60).
 * Returns the number of entries written, or -1 on bad arguments. */
int decdab_fdk_huffman_scalefactor(unsigned int *codes, unsigned char *lens,
                                   int capacity) {
  if (codes == NULL || lens == NULL || capacity < 121) return -1;
  for (int i = 0; i < 121; i++) {
    codes[i] = (unsigned int)FDKaacEnc_huff_ctabscf[i];
    lens[i] = FDKaacEnc_huff_ltabscf[i];
  }
  return 121;
}

} /* extern "C" */
