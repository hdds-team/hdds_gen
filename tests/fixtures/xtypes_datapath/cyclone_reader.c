/* Foreign reader for the hdds_gen data path: Cyclone DDS 0.11 validates
 * (dds_stream_normalize) and reads (dds_stream_read_sample) a body another
 * implementation serialized, then serializes the sample again. Input lines:
 * "<Type> <xcdr1|xcdr2> <hex body>"; output: "<Type> <ver> OK <hex>" with
 * Cyclone's own serialization of what it read, or "<Type> <ver> REJECT".
 *
 * With the argument "spec", each XCDR1 body is rewritten as DDS-XTypes 1.3
 * 7.4.1.2.2 specifies the extended parameter header, where Cyclone DDS 0.11
 * departs from it: the member length is the exact length of the member
 * (Cyclone counts the padding up to the next header) and the member header
 * carries FLAG_2, must understand, for a key member (Cyclone never sets it).
 * The exact length is the smallest one Cyclone's own reader accepts with
 * the same sample read; the key members are given as a list of member ids
 * after the hex ("-" for none). Output: "<Type> xcdr1 spec=<hex>".
 * Build: idlc corpus.idl; cc -I. -I<cyclone>/include cyclone_reader.c
 * corpus.c -L<cyclone>/lib -lddsc */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "dds/dds.h"
#include "dds/cdr/dds_cdrstream.h"
#include "corpus.h"

static const struct { const char *name; const dds_topic_descriptor_t *desc; } types[] = {
  { "FinalPrims", &FinalPrims_desc },
  { "Inner8", &Inner8_desc },
  { "NestFinal", &NestFinal_desc },
  { "InnerL", &InnerL_desc },
  { "NestOdd", &NestOdd_desc },
  { "AppStruct", &AppStruct_desc },
  { "NestApp", &NestApp_desc },
  { "MutDefault", &MutDefault_desc },
  { "MutIds", &MutIds_desc },
  { "MutHash", &MutHash_desc },
  { "MutHashid", &MutHashid_desc },
  { "MutPrims", &MutPrims_desc },
  { "MutCompact2", &MutCompact2_desc },
  { "NestMut", &NestMut_desc },
  { "MutOpt", &MutOpt_desc },
  { "BaseF", &BaseF_desc },
  { "DerivedF", &DerivedF_desc },
  { "BaseA", &BaseA_desc },
  { "DerivedA", &DerivedA_desc },
  { "BaseM", &BaseM_desc },
  { "DerivedM", &DerivedM_desc },
  { "Enums", &Enums_desc },
  { "MutEnums", &MutEnums_desc },
  { "AppEnums", &AppEnums_desc },
  { "Seqs", &Seqs_desc },
  { "SeqStruct", &SeqStruct_desc },
  { "SeqApp", &SeqApp_desc },
  { "SeqStr", &SeqStr_desc },
  { "Arrs", &Arrs_desc },
  { "ArrStruct", &ArrStruct_desc },
  { "UL", &UL_desc },
  { "HoldU", &HoldU_desc },
  { "UA", &UA_desc },
  { "HoldUA", &HoldUA_desc },
  { "UE", &UE_desc },
  { "HoldUE", &HoldUE_desc },
  { "BStr", &BStr_desc },
  { "Keyed", &Keyed_desc },
  { "KeyedApp", &KeyedApp_desc },
  { "FinalOpt", &FinalOpt_desc },
  { "AppOpt", &AppOpt_desc },
  { "MutMix", &MutMix_desc },
  { "SeqEnum", &SeqEnum_desc },
  { "WStr", &WStr_desc },
  { "Aliases", &Aliases_desc },
  { "ArrStr", &ArrStr_desc },
  { "MutLC", &MutLC_desc },
  { "MutLC2", &MutLC2_desc },
  { "US", &US_desc },
  { "HoldUS", &HoldUS_desc },
  { "SeqChar", &SeqChar_desc },
  { "MutSeqChar", &MutSeqChar_desc },
  { "MutW", &MutW_desc },
  { "AppW", &AppW_desc },
  { "NestAppW", &NestAppW_desc },
  { "UB2", &UB2_desc },
  { "HoldUB2", &HoldUB2_desc },
  { "UE2", &UE2_desc },
  { "HoldUE2", &HoldUE2_desc },
  { "UC2", &UC2_desc },
  { "HoldUC2", &HoldUC2_desc },
};

static int unhex(const char *s, unsigned char *out, size_t max) {
  size_t n = strlen(s) / 2, i;
  if (n > max) return -1;
  for (i = 0; i < n; i++) { unsigned v; if (sscanf(s + 2 * i, "%2x", &v) != 1) return -1; out[i] = (unsigned char)v; }
  return (int)n;
}

/* Whether Cyclone reads `in` (n bytes) as a sample whose serialization is
 * `expect`. */
static int reads_as(const dds_topic_descriptor_t *desc, const unsigned char *in, uint32_t n, const unsigned char *expect, uint32_t expect_n) {
  static unsigned char buf[8192], out[8192];
  struct dds_cdrstream_desc cd;
  uint32_t actual = 0;
  void *sample;
  dds_istream_t is;
  dds_ostream_t os;
  int ok = 0;
  memcpy(buf, in, n);
  dds_cdrstream_desc_from_topic_desc(&cd, desc);
  if (dds_stream_normalize(buf, n, false, 1, &cd, false, &actual)) {
    sample = calloc(1, desc->m_size);
    dds_istream_init(&is, n, buf, 1);
    dds_stream_read_sample(&is, sample, &dds_cdrstream_default_allocator, &cd);
    memset(out, 0, sizeof out);
    os.m_buffer = out; os.m_size = (uint32_t)sizeof out; os.m_index = 0; os.m_xcdr_version = 1;
    if (dds_stream_write_sampleLE((dds_ostreamLE_t *)&os, &dds_cdrstream_default_allocator, sample, &cd))
      ok = os.m_index == expect_n && memcmp(out, expect, expect_n) == 0;
    dds_stream_free_sample(sample, &dds_cdrstream_default_allocator, cd.ops.ops);
    free(sample);
  }
  dds_cdrstream_desc_fini(&cd, &dds_cdrstream_default_allocator);
  return ok;
}

static int spec(void) {
  char line[16384], name[128], ver[16], hex[16000], keys[256];
  while (fgets(line, sizeof line, stdin)) {
    static unsigned char orig[8192], cur[8192];
    const dds_topic_descriptor_t *desc = NULL;
    size_t t;
    int n, i;
    if (sscanf(line, "%127s %15s %15999s %255s", name, ver, hex, keys) != 4) continue;
    for (t = 0; t < sizeof types / sizeof types[0]; t++) if (strcmp(types[t].name, name) == 0) desc = types[t].desc;
    n = unhex(hex, orig, sizeof orig);
    if (!desc || n < 0 || strcmp(ver, "xcdr1") != 0) { printf("%s %s BADINPUT\n", name, ver); continue; }
    memcpy(cur, orig, (size_t)n);
    if (!reads_as(desc, cur, (uint32_t)n, orig, (uint32_t)n)) { printf("%s %s REJECT\n", name, ver); continue; }
    for (i = 0; i + 12 <= n; i += 4) {
      uint32_t id, len, k;
      if (!(cur[i] == 0x01 && cur[i + 1] == 0x7f && cur[i + 2] == 0x08 && cur[i + 3] == 0x00)) continue;
      memcpy(&id, cur + i + 4, 4);
      memcpy(&len, cur + i + 8, 4);
      /* smallest length Cyclone still reads as the same sample */
      /* (never below 1: Cyclone 0.11 normalizes a zero-length boolean member
         and then crashes reading it) */
      for (k = 0; k < 4 && len > 1; k++) {
        uint32_t shorter = len - 1;
        memcpy(cur + i + 8, &shorter, 4);
        if (!reads_as(desc, cur, (uint32_t)n, orig, (uint32_t)n)) { memcpy(cur + i + 8, &len, 4); break; }
        len = shorter;
      }
      if (strcmp(keys, "-") != 0) {
        char list[256], *tok;
        strcpy(list, keys);
        for (tok = strtok(list, ","); tok; tok = strtok(NULL, ","))
          if ((uint32_t)strtoul(tok, NULL, 0) == (id & 0x0fffffffu)) { id |= 0x40000000u; memcpy(cur + i + 4, &id, 4); }
      }
    }
    printf("%s %s spec=", name, ver);
    for (i = 0; i < n; i++) printf("%02x", cur[i]);
    printf("\n");
  }
  return 0;
}

int main(int argc, char **argv) {
  char line[16384], name[128], ver[16], hex[16000];
  if (argc > 1 && strcmp(argv[1], "spec") == 0) return spec();
  while (fgets(line, sizeof line, stdin)) {
    static unsigned char in[8192], out[8192];
    const dds_topic_descriptor_t *desc = NULL;
    struct dds_cdrstream_desc cd;
    uint32_t v, actual = 0, i;
    int n;
    size_t t;
    void *sample;
    dds_istream_t is;
    dds_ostream_t os;
    if (sscanf(line, "%127s %15s %15999s", name, ver, hex) != 3) continue;
    for (t = 0; t < sizeof types / sizeof types[0]; t++) if (strcmp(types[t].name, name) == 0) desc = types[t].desc;
    if (!desc) { printf("%s %s UNKNOWN\n", name, ver); continue; }
    v = strcmp(ver, "xcdr1") == 0 ? 1 : 2;
    n = unhex(hex, in, sizeof in);
    if (n < 0) { printf("%s %s BADHEX\n", name, ver); continue; }
    dds_cdrstream_desc_from_topic_desc(&cd, desc);
    if (!dds_stream_normalize(in, (uint32_t)n, false, v, &cd, false, &actual)) { printf("%s %s REJECT\n", name, ver); dds_cdrstream_desc_fini(&cd, &dds_cdrstream_default_allocator); continue; }
    sample = calloc(1, desc->m_size);
    dds_istream_init(&is, (uint32_t)n, in, v);
    dds_stream_read_sample(&is, sample, &dds_cdrstream_default_allocator, &cd);
    memset(out, 0, sizeof out);
    os.m_buffer = out; os.m_size = (uint32_t)sizeof out; os.m_index = 0; os.m_xcdr_version = v;
    if (!dds_stream_write_sampleLE((dds_ostreamLE_t *)&os, &dds_cdrstream_default_allocator, sample, &cd)) { printf("%s %s REWRITE_FAIL\n", name, ver); }
    else { printf("%s %s OK used=%u ", name, ver, actual); for (i = 0; i < os.m_index; i++) printf("%02x", out[i]); printf("\n"); }
    dds_stream_free_sample(sample, &dds_cdrstream_default_allocator, cd.ops.ops);
    free(sample);
    dds_cdrstream_desc_fini(&cd, &dds_cdrstream_default_allocator);
  }
  return 0;
}
