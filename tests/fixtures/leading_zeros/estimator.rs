mod estimator;
fn c0()->bool{estimator::estimate_scaling_factor(1u64,0i16)==0i16}
fn c1()->bool{estimator::estimate_scaling_factor(1u64,-100i16)==-31i16}
fn c2()->bool{estimator::estimate_scaling_factor(1u64,100i16)==30i16}
fn c3()->bool{estimator::estimate_scaling_factor(2u64,0i16)==0i16}
fn c4()->bool{estimator::estimate_scaling_factor(3u64,-1i16)==0i16}
fn c5()->bool{estimator::estimate_scaling_factor(2147483648u64,-31i16)==0i16}
fn c6()->bool{estimator::estimate_scaling_factor(4294967296u64,10i16)==12i16}
fn c7()->bool{estimator::estimate_scaling_factor(9223372036854775808u64,-100i16)==-12i16}
fn c8()->bool{estimator::estimate_scaling_factor(18446744073709551615u64,100i16)==49i16}
fn sweep(mant:u64,bits:i64)->bool{let mut exp=-1074i16;while exp<=1024i16{let expected=(((bits+exp as i64)*1292913986i64)>>32) as i16;if estimator::estimate_scaling_factor(mant,exp)!=expected{return false;}exp=exp+1i16;}true}
fn main(){if c0() && c1() && c2() && c3() && c4() && c5() && c6() && c7() && c8() && sweep(1u64,0i64) && sweep(2u64,1i64) && sweep(3u64,2i64) && sweep(2147483648u64,31i64) && sweep(4294967296u64,32i64) && sweep(9223372036854775808u64,63i64) && sweep(18446744073709551615u64,64i64){println!("ok");}}
