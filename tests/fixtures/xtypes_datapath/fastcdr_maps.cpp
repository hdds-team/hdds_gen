// Fast CDR 2.3.3: XCDR1 / XCDR2 serialization of a final struct holding
// a long and two maps (map<long, string>, map<long, long>), as Fast DDS
// writes them. Build: g++ -std=c++17 -I<fastdds>/include fastcdr_maps.cpp
// -L<fastdds>/lib -lfastcdr. The `one` lines use single-entry maps.
#include <fastcdr/Cdr.h>
#include <fastcdr/FastBuffer.h>
#include <cstdio>
#include <map>
#include <string>
using namespace eprosima::fastcdr;
static void run(CdrVersion v, const char* name) {
    char buf[256] = {0};
    FastBuffer fb(buf, sizeof buf);
    Cdr cdr(fb, Cdr::LITTLE_ENDIANNESS, v);
    cdr.set_encoding_flag(v == CdrVersion::XCDRv2 ? EncodingAlgorithmFlag::PLAIN_CDR2 : EncodingAlgorithmFlag::PLAIN_CDR);
    std::map<int32_t, std::string> m{{1, "a"}, {2, "bc"}};
    std::map<int32_t, int32_t> pm{{3, 4}};
    cdr << int32_t(7) << m << pm;
    printf("%s ", name);
    for (size_t i = 0; i < cdr.get_serialized_data_length(); i++) printf("%02x", (unsigned char)buf[i]);
    printf("\n");
}
int main() { run(CdrVersion::XCDRv1, "xcdr1"); run(CdrVersion::XCDRv2, "xcdr2"); }
