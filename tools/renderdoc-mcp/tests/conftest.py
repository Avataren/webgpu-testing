"""Make RenderDoc's Python module importable; skip the tests when it is not built."""

import os
import sys
from pathlib import Path

module_dir = os.environ.setdefault("RENDERDOC_MODULE_PATH",
                                   str(Path.home() / "src/renderdoc/build/lib"))
if module_dir not in sys.path:
    sys.path.insert(0, module_dir)

try:
    import renderdoc  # noqa: F401
except ImportError:
    print(f"RenderDoc's Python module is not in {module_dir}; skipping the tests "
          "(build it with build_renderdoc_module.sh)")
    collect_ignore_glob = ["test_*.py"]
