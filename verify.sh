# cargo kani --exact --harness kademlia::verification::distance_inversion \
cargo kani \
    --enable-unstable $@ \
    --cbmc-args \
    --unwindset memcmp.0:257 \
    --unwindset _ZN4core5slice6memchr12memchr_naive17h64b537d458d6e690E.0:257 \
    --unwindset _ZN4core5slice6memchr14memchr_aligned17h75bd820b1dfabaafE.0:257 \
    --unwindset _RNvMs2m_CscBgMDJYxYlU_15primitive_typesNtB6_4U25618from_little_endian.0:257 \
    --unwindset _RNvXs2I_CscBgMDJYxYlU_15primitive_typesNtB6_4U256NtNtNtCsiCvmSzcCjPe_4core3ops3bit6BitXor6bitxor.0:257 \
    --unwindset _RNvMs2m_CscBgMDJYxYlU_15primitive_typesNtB6_4U25613leading_zeros.0:257 \
    --unwindset _RINvXs2T_NtNtCsiCvmSzcCjPe_4core5slice4iterINtB7_4IterINtNtCskqTXbRBwjM9_8augustus8kademlia10PeerRecorduuEENtNtNtNtBb_4iter6traits8iterator8Iterator8positionNCNvMs1_BT_INtBT_7BucketsuuKj8_E6insert0EBV_.0:21 \
    --unwindset _RINvXs2T_NtNtCsiCvmSzcCjPe_4core5slice4iterINtB7_4IterINtNtCskqTXbRBwjM9_8augustus8kademlia10PeerRecorduuEENtNtNtNtBb_4iter6traits8iterator8Iterator8positionNCNvMs1_BT_INtBT_7BucketsuuKj8_E6inserts_0EBV_.0:21 \
    --unwindset _RINvNtCsiCvmSzcCjPe_4core5array18try_from_fn_erasedINtNtCskqTXbRBwjM9_8augustus8kademlia6BucketuuEINtNtNtB4_3ops9try_trait17NeverShortCircuitBN_ENCINvMB1B_B1y_10wrap_mut_1jNCNvMs1_BQ_INtBQ_7BucketsuuKj8_E3new0E0EBS_.0:9 \
    --unwindset _RNvNtNtCskqTXbRBwjM9_8augustus8kademlia12verification15ordered_closest.0:3