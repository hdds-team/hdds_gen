/* Foreign oracle for the hdds_gen data path: Cyclone DDS 0.11 serializes one
 * sample of each type of corpus.idl under XCDR1 and XCDR2 (writer and reader
 * in one participant, local delivery; the serialized form is the reader's
 * serdata). Output: "<sample> xcdr1 <hex>" / "<sample> xcdr2 <hex>", the hex
 * including the 4-byte encapsulation header. Build:
 *   idlc -Wno-inherit-appendable corpus.idl
 *   cc -I. -I/opt/cyclonedds/include probe.c corpus.c -L/opt/cyclonedds/lib -lddsc */
#include <stdio.h>
#include <string.h>
#include "dds/dds.h"
#include "dds/ddsi/ddsi_serdata.h"
#include "dds/cdr/dds_cdrstream.h"
#include "corpus.h"

static dds_entity_t pp;

static void stream(const dds_topic_descriptor_t *desc, const void *sample, uint32_t ver, unsigned char fill, unsigned char *out, uint32_t *n) {
  static unsigned char buf[4096];
  struct dds_cdrstream_desc cd;
  dds_ostream_t os;
  memset(buf, fill, sizeof buf);
  dds_cdrstream_desc_from_topic_desc(&cd, desc);
  os.m_buffer = buf; os.m_size = (uint32_t)sizeof buf; os.m_index = 0; os.m_xcdr_version = ver;
  if (!dds_stream_write_sampleLE((dds_ostreamLE_t *)&os, &dds_cdrstream_default_allocator, sample, &cd)) { *n = 0; return; }
  *n = os.m_index;
  memcpy(out, os.m_buffer, os.m_index);
  dds_cdrstream_desc_fini(&cd, &dds_cdrstream_default_allocator);
}

static void probe(const char *name, const dds_topic_descriptor_t *desc, const void *sample) {
  dds_data_representation_id_t reprs[2] = { DDS_DATA_REPRESENTATION_XCDR1, DDS_DATA_REPRESENTATION_XCDR2 };
  int r;
  for (r = 0; r < 2; r++) {
    dds_entity_t tp, rd, wr;
    dds_qos_t *qos = dds_create_qos();
    struct ddsi_serdata *sd[1] = { NULL };
    dds_sample_info_t si[1];
    dds_return_t n = 0;
    int tries;
    char tname[128], hdr[16], note[64];
    unsigned char a[4096], b[4096], ser[4096];
    uint32_t na, nb, sz = 0, i;
    stream(desc, sample, (uint32_t)(r + 1), 0x00, a, &na);
    stream(desc, sample, (uint32_t)(r + 1), 0xff, b, &nb);
    strcpy(hdr, "-"); strcpy(note, "");
    snprintf(tname, sizeof tname, "%s_%d", name, r);
    dds_qset_data_representation(qos, 1, &reprs[r]);
    tp = dds_create_topic(pp, desc, tname, NULL, NULL);
    rd = dds_create_reader(pp, tp, qos, NULL);
    wr = dds_create_writer(pp, tp, qos, NULL);
    dds_delete_qos(qos);
    if (rd < 0 || wr < 0) {
      snprintf(note, sizeof note, " endpoint-refused:%s", dds_strretcode(wr < 0 ? wr : rd));
    } else if ((n = dds_write(wr, sample)) >= 0) {
      for (tries = 0; tries < 200; tries++) {
        n = dds_takecdr(rd, sd, 1, si, DDS_ANY_STATE);
        if (n > 0) break;
        dds_sleepfor(DDS_MSECS(5));
      }
      if (n > 0) {
        sz = ddsi_serdata_size(sd[0]);
        ddsi_serdata_to_ser(sd[0], 0, sz, ser);
        ddsi_serdata_unref(sd[0]);
        snprintf(hdr, sizeof hdr, "%02x%02x%02x%02x", ser[0], ser[1], ser[2], ser[3]);
        /* the serdata body must equal the stream body outside padding */
        if (sz - 4 - (ser[3] & 3) != na) snprintf(note, sizeof note, " SERDATA-LEN-MISMATCH");
        else for (i = 0; i < na; i++) if (a[i] == b[i] && ser[4 + i] != a[i]) { snprintf(note, sizeof note, " SERDATA-BYTE-MISMATCH@%u", i); break; }
      }
    }
    dds_delete(tp);
    if (na == 0 || na != nb) { printf("%s xcdr%d FAIL stream\n", name, r + 1); continue; }
    printf("%s xcdr%d hdr=%s body=", name, r + 1, hdr);
    for (i = 0; i < na; i++) printf("%02x", a[i]);
    printf(" pad=");
    for (i = 0; i < na; i++) putchar(a[i] == b[i] ? '.' : 'p');
    printf("%s\n", note);
  }
}

#define SEQ(s, arr) do { (s)._maximum = (s)._length = (uint32_t)(sizeof(arr) / sizeof((arr)[0])); (s)._buffer = (arr); (s)._release = false; } while (0)

int main(void) {
  pp = dds_create_participant(70, NULL, NULL);
  if (pp < 0) { printf("participant: %s\n", dds_strretcode(pp)); return 2; }
  { FinalPrims v = { 0x11, 0x0102030405060708LL, -2, 1.5, true, 0xAABBCCDDu }; probe("FinalPrims", &FinalPrims_desc, &v); }
  { NestFinal v = { 1, { 0x1122334455667788LL, 7 }, 9 }; probe("NestFinal", &NestFinal_desc, &v); }
  { NestOdd v = { 1, { 0x01020304 }, 5 }; probe("NestOdd", &NestOdd_desc, &v); }
  { NestApp v = { 1, { 2, "hi", 3 }, 4 }; probe("NestApp", &NestApp_desc, &v); }
  { MutDefault v = { 1, "abc", 2.0, 7 }; probe("MutDefault", &MutDefault_desc, &v); }
  { MutIds v = { 1, 2, 3, 4 }; probe("MutIds", &MutIds_desc, &v); }
  { MutHash v = { 1, 2, 3, 4 }; probe("MutHash", &MutHash_desc, &v); }
  { MutHashid v = { 1, 2, 3 }; probe("MutHashid", &MutHashid_desc, &v); }
  { MutPrims v = { 1, 2, 3 }; probe("MutPrims", &MutPrims_desc, &v); }
  { MutCompact2 v = { 1, 2, 3 }; probe("MutCompact2", &MutCompact2_desc, &v); }
  { NestMut v = { 1, { 1, 2, 3 }, 2 }; probe("NestMut", &NestMut_desc, &v); }
  { int32_t a = 5; MutOpt v = { &a, "x", 6 }; probe("MutOpt_all", &MutOpt_desc, &v); }
  { MutOpt v = { NULL, NULL, 6 }; probe("MutOpt_none", &MutOpt_desc, &v); }
  { DerivedF v = { { 1, 2.5 }, 3, 4 }; probe("DerivedF", &DerivedF_desc, &v); }
  { DerivedA v = { { 1 }, "n" }; probe("DerivedA", &DerivedA_desc, &v); }
  { DerivedM v = { { 1, "t" }, 2, 3, 4 }; probe("DerivedM", &DerivedM_desc, &v); }
  { Enums v = { E8_B, E16_B, E32_C, 0xEE, E16_A, E8_C }; probe("Enums", &Enums_desc, &v); }
  { MutEnums v = { E8_C, E16_B, E32_B }; probe("MutEnums", &MutEnums_desc, &v); }
  { AppEnums v = { 1, E16_B }; probe("AppEnums", &AppEnums_desc, &v); }
  { int64_t u[] = { 1, 2 }; double bd[] = { 0.5 }; int16_t s[] = { 3, 4, 5 }; int64_t bl[] = { 6 };
    Seqs v; memset(&v, 0, sizeof v); v.p = 1; SEQ(v.u, u); SEQ(v.bd, bd); v.q = 2; SEQ(v.s, s); SEQ(v.bl, bl); probe("Seqs", &Seqs_desc, &v); }
  { Inner8 e[] = { { 1, 2 }, { 3, 4 } }; SeqStruct v; v.p = 1; SEQ(v.v, e); probe("SeqStruct", &SeqStruct_desc, &v); }
  { AppStruct e[] = { { 1, "a", 2 }, { 3, "", 4 } }; SeqApp v; v.p = 1; SEQ(v.v, e); probe("SeqApp", &SeqApp_desc, &v); }
  { char *s1[] = { "ab", "c" }; char s2[][5] = { "wxyz" }; SeqStr v; v.p = 1; SEQ(v.v, s1); SEQ(v.b, s2); probe("SeqStr", &SeqStr_desc, &v); }
  { Arrs v = { 1, { 1, 2, 3 }, { { 1, 2, 3 }, { 4, 5, 6 } }, { { 1, 2 }, { 3, 4 } } }; probe("Arrs", &Arrs_desc, &v); }
  { ArrStruct v = { 1, { { 1, 2 }, { 3, 4 } } }; probe("ArrStruct", &ArrStruct_desc, &v); }
  { HoldU v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 1; v.u._u.a = 0x0A; v.z = 9; probe("HoldU_1", &HoldU_desc, &v); }
  { HoldU v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 2; strcpy(v.u._u.s, "hello"); v.z = 9; probe("HoldU_2", &HoldU_desc, &v); }
  { HoldU v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 3; v.u._u.d = 0.25; v.z = 9; probe("HoldU_3", &HoldU_desc, &v); }
  { HoldU v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 9; v.u._u.o = 0x7F; v.z = 9; probe("HoldU_def", &HoldU_desc, &v); }
  { HoldUA v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 1; v.u._u.a = 5; probe("HoldUA_1", &HoldUA_desc, &v); }
  { HoldUA v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 2; strcpy(v.u._u.s, "ab"); probe("HoldUA_2", &HoldUA_desc, &v); }
  { HoldUE v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = E8_A; v.u._u.a = 6; v.z = 2; probe("HoldUE_A", &HoldUE_desc, &v); }
  { HoldUE v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = E8_B; v.u._u.b = 7; v.z = 2; probe("HoldUE_B", &HoldUE_desc, &v); }
  { BStr v = { 1, "abcdefgh", 2 }; probe("BStr", &BStr_desc, &v); }
  { Keyed v = { 1, "nm", 3 }; probe("Keyed", &Keyed_desc, &v); }
  { KeyedApp v = { 1, 2 }; probe("KeyedApp", &KeyedApp_desc, &v); }
  { FinalOpt v; int32_t a = 5; int64_t b = 6; v.t = 1; v.a = &a; v.b = &b; v.u = 2; probe("FinalOpt_all", &FinalOpt_desc, &v); }
  { FinalOpt v; v.t = 1; v.a = NULL; v.b = NULL; v.u = 2; probe("FinalOpt_none", &FinalOpt_desc, &v); }
  { AppOpt v; int32_t a = 7; v.t = 1; v.a = &a; probe("AppOpt_all", &AppOpt_desc, &v); }
  { AppOpt v; v.t = 1; v.a = NULL; probe("AppOpt_none", &AppOpt_desc, &v); }
  { int32_t sl[] = { 1, 2 }; char *ss[] = { "a" }; E8 se[] = { E8_B, E8_C }; MutMix v; memset(&v, 0, sizeof v);
    SEQ(v.sl, sl); SEQ(v.ss, ss); v.fin.x = 3; v.fin.y = 4; v.app.a = 5; v.app.s = "b"; v.app.b = 6; v.mut.x = 7; v.mut.y = 8; v.mut.z = 9;
    v.un._d = 2; strcpy(v.un._u.s, "u"); v.arr[0] = 10; v.arr[1] = 11; strcpy(v.bs, "xy"); v.en = E16_B; SEQ(v.se, se); probe("MutMix", &MutMix_desc, &v); }
  { E8 a[] = { E8_C, E8_A, E8_B }; E32 b[] = { E32_B }; SeqEnum v; v.p = 1; SEQ(v.a, a); SEQ(v.b, b); v.arr[0] = E8_B; v.arr[1] = E8_C; v.arr[2] = E8_A; probe("SeqEnum", &SeqEnum_desc, &v); }
  { WStr v; v.p = 1; v.c = L'A'; v.s = L"hé"; v.d = 0x263A; probe("WStr", &WStr_desc, &v); }
  { int32_t l[] = { 7, 8 }; Aliases v; v.p = 1; v.m = BM32_0 | BM32_5; v.n = BM8_1; SEQ(v.l, l); v.ia.x = 9; v.ia.y = 10; probe("Aliases", &Aliases_desc, &v); }
  { ArrStr v; v.p = 1; v.a[0] = "ab"; v.a[1] = ""; strcpy(v.b[0], "xyz"); strcpy(v.b[1], "q"); probe("ArrStr", &ArrStr_desc, &v); }
  { HoldU v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 0; v.u._u.o = 0x7F; v.z = 9; probe("HoldU_def0", &HoldU_desc, &v); }
  { uint8_t s1[] = { 1, 2, 3 }; int16_t s2[] = { 4 }; int32_t s4[] = { 5, 6 }; int64_t s8[] = { 7 }; double sd[] = { 0.5, 1.0 }; bool sb[] = { true };
    Inner8 si[] = { { 1, 2 } }; MutLC v; memset(&v, 0, sizeof v); SEQ(v.s1, s1); SEQ(v.s2, s2); SEQ(v.s4, s4); SEQ(v.s8, s8); SEQ(v.sd, sd); SEQ(v.sb, sb);
    v.a1[0] = 1; v.a1[1] = 2; v.a1[2] = 3; v.a2[0] = 4; v.a2[1] = 5; v.a4[0] = 6; v.a4[1] = 7; v.s = "st"; SEQ(v.si, si); v.e = E32_C; v.m = BM32_1; v.o = NULL; v.a8[0] = 8; v.b = true;
    v.s0._length = v.s0._maximum = 0; v.s0._buffer = NULL; probe("MutLC", &MutLC_desc, &v); }
  { BM8 sbm[] = { BM8_1 }; int32_t ls[] = { 3 }; MutLC2 v; memset(&v, 0, sizeof v); v.ai[0].x = 1; v.ai[0].y = 2; v.ae[0] = E8_B; v.ae[1] = E8_C; v.sa[0] = "s";
    v.bm = BM8_0; SEQ(v.sbm, sbm); SEQ(v.ls, ls); v.ia.x = 4; v.ia.y = 5; probe("MutLC2", &MutLC2_desc, &v); }
  { HoldUS v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 1; v.u._u.a = 0x11; v.q = 2; probe("HoldUS_1", &HoldUS_desc, &v); }
  { HoldUS v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 2; v.u._u.b = 0x2233; v.q = 2; probe("HoldUS_2", &HoldUS_desc, &v); }
  { HoldUS v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 3; v.u._u.c = 0x44556677; v.q = 2; probe("HoldUS_3", &HoldUS_desc, &v); }
  { char c[] = { 'x', 'y' }; wchar_t w[] = { L'A', L'B' }; SeqChar v; v.p = 1; SEQ(v.c, c); SEQ(v.w, w); strcpy(v.s, "ab"); probe("SeqChar", &SeqChar_desc, &v); }
  { char c[] = { 'q' }; wchar_t w[] = { L'Z' }; MutSeqChar v; SEQ(v.c, c); SEQ(v.w, w); v.z = 5; probe("MutSeqChar", &MutSeqChar_desc, &v); }
  { MutW v = { 1, 2, "w" }; probe("MutW", &MutW_desc, &v); }
  { AppW v = { 1, 2 }; probe("AppW", &AppW_desc, &v); }
  { NestAppW v = { 1, { 3, 4 }, 5 }; probe("NestAppW", &NestAppW_desc, &v); }
  { HoldUB2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = true; v.u._u.t = 0x11; probe("HB_T", &HoldUB2_desc, &v); }
  { HoldUB2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = false; v.u._u.f = 0x22; probe("HB_F", &HoldUB2_desc, &v); }
  { HoldUE2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = lm2_A; v.u._u.a = 6; probe("HE_A", &HoldUE2_desc, &v); }
  { HoldUE2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = lm2_B; v.u._u.b = 7; probe("HE_B", &HoldUE2_desc, &v); }
  { HoldUC2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 'a'; v.u._u.x = 5; probe("HC_A", &HoldUC2_desc, &v); }
  { HoldUC2 v; memset(&v, 0, sizeof v); v.p = 1; v.u._d = 'b'; v.u._u.y = 7; probe("HC_B", &HoldUC2_desc, &v); }
  dds_delete(pp);
  return 0;
}
