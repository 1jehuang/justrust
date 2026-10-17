use justrust_engine::compiler;

#[test]
fn source_defaults_keep_shared_checked_native_facts() {
    let source = "struct S{x:i32}trait V:Sized{fn required(self)->i32;fn value(self)->i32{self.required()+1}}impl V for S{fn required(self)->i32{self.x}}fn call<T:V>(x:T)->i32{x.value()}fn main(){let n=call(S{x:6});}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.into(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for cfg in &compiled.control_flow {
                cfg.validate().unwrap();
            }
        }
    }
}

#[test]
fn unused_and_overridden_default_failures_reach_ownership_analysis() {
    for source in [
        "trait V:Sized{fn duplicate(self)->(Self,Self){(self,self)}}fn main(){}",
        "struct S{x:i32}trait V:Sized{fn required(self)->i32;fn value(self)->i32{self.required()+self.required()}}impl V for S{fn required(self)->i32{self.x}fn value(self)->i32{7}}fn main(){}",
        "struct S{x:i32}trait V{fn required(&mut self)->i32;fn value(&mut self)->i32{let a=&mut *self;let b=&mut *self;a.required()+b.required()}}impl V for S{fn required(&mut self)->i32{self.x}fn value(&mut self)->i32{7}}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
                assert!(
                    failure
                        .analyses
                        .iter()
                        .any(|(_, analysis)| !analysis.passed()),
                    "default rejection must reach ownership analysis: {}",
                    failure.error
                );
            }
        }
    }
}
