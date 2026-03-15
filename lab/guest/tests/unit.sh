#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
guest_root="$(cd -- "${script_dir}/.." && pwd)"

# shellcheck source=/dev/null
source "${guest_root}/lib/common.sh"
# shellcheck source=/dev/null
source "${guest_root}/lib/topology.sh"
# shellcheck source=/dev/null
source "${guest_root}/lib/validate.sh"

assert_eq() {
  local got="$1" want="$2" name="$3"
  if [[ "${got}" != "${want}" ]]; then
    echo "not ok: ${name} (got=${got} want=${want})" >&2
    exit 1
  fi
  echo "ok: ${name}"
}

test_tcpdump_parse() {
  local line="IP 100.64.0.2.12345 > 100.64.0.10.41000: UDP, length 2"
  assert_eq "$(tcpdump_parse_src_port "${line}")" "12345" "tcpdump_parse_src_port"
  assert_eq "$(tcpdump_parse_dst "${line}")" "100.64.0.10.41000" "tcpdump_parse_dst"
}

test_expecteds() {
  assert_eq "$(expected_mapping nat1)" "EIM" "expected_mapping nat1"
  assert_eq "$(expected_mapping nat2)" "EIM" "expected_mapping nat2"
  assert_eq "$(expected_mapping nat3)" "EIM" "expected_mapping nat3"
  assert_eq "$(expected_mapping nat4-regular)" "APDM" "expected_mapping nat4-regular"
  assert_eq "$(expected_mapping nat4-irregular)" "APDM" "expected_mapping nat4-irregular"

  assert_eq "$(expected_filtering nat1)" "EIF" "expected_filtering nat1"
  assert_eq "$(expected_filtering nat2)" "ADF" "expected_filtering nat2"
  assert_eq "$(expected_filtering nat3)" "APDF" "expected_filtering nat3"
  assert_eq "$(expected_filtering nat4-regular)" "APDF" "expected_filtering nat4-regular"
  assert_eq "$(expected_filtering nat4-irregular)" "APDF" "expected_filtering nat4-irregular"
}

test_nat_label_from() {
  assert_eq "$(nat_label_from EIM EIF)" "NAT1" "nat_label_from NAT1"
  assert_eq "$(nat_label_from EIM ADF)" "NAT2" "nat_label_from NAT2"
  assert_eq "$(nat_label_from EIM APDF)" "NAT3" "nat_label_from NAT3"
  assert_eq "$(nat_label_from APDM APDF)" "NAT4" "nat_label_from NAT4"
  assert_eq "$(nat_label_from ADM EIF)" "NAT-OTHER" "nat_label_from NAT-OTHER"
}

test_mapping_classify_from_ports() {
  assert_eq "$(mapping_classify_from_ports 5000 5000)" "EIM" "mapping_classify_from_ports EIM"
  assert_eq "$(mapping_classify_from_ports 40001 40002)" "APDM" "mapping_classify_from_ports APDM"
}

test_filtering_classify_from_flags() {
  assert_eq "$(filtering_classify_from_flags 1 0)" "EIF" "filtering_classify EIF"
  assert_eq "$(filtering_classify_from_flags 0 1)" "ADF" "filtering_classify ADF"
  assert_eq "$(filtering_classify_from_flags 0 0)" "APDF" "filtering_classify APDF"
}

main() {
  test_tcpdump_parse
  test_expecteds
  test_nat_label_from
  test_mapping_classify_from_ports
  test_filtering_classify_from_flags
  echo "all unit tests passed"
}

main "$@"

