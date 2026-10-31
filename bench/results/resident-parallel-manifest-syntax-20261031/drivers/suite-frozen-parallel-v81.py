#!/usr/bin/env python3
"""Run accepted suite CLI with exact private frozen v81 factor adapter, not live rejected code."""
import importlib.util
import runpy
import sys
from pathlib import Path
bench=Path('<repository>/bench')
private=Path('<native-root>/parallel-manifest-harness-v81/candidates/bench/resident_suite.py')
sys.path.insert(0,str(bench))
spec=importlib.util.spec_from_file_location('resident_suite', private)
module=importlib.util.module_from_spec(spec)
sys.modules['resident_suite']=module
spec.loader.exec_module(module)
runpy.run_path(str(bench/'suite.py'),run_name='__main__')
