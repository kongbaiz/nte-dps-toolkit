from __future__ import annotations

import json
import sys
from pathlib import Path


root = Path(sys.argv[1]).resolve()
page = (root / "frontend/src/features/history/history-page.tsx").read_text(encoding="utf-8")
client = (root / "frontend/src/lib/tauri/history-client.ts").read_text(encoding="utf-8")
contract = (root / "src-tauri/src/contract/history.rs").read_text(encoding="utf-8")
view_model = (root / "frontend/src/features/history/history-view-model.ts").read_text(encoding="utf-8")

result = {
    "browser_file_input": 'type="file"' in page,
    "browser_blob_export": "URL.createObjectURL" in page or "new Blob" in page,
    "native_import_command": "import_history_record_file" in client,
    "native_export_command": "export_history_record_file" in client,
    "capture_json_enabled": "onClick={() => void importCaptureJson()}" in page,
    "history_context_menu": "onContextMenu" in page and "HistoryContextMenu" in page,
    "separate_compare_warnings": "historyComparisonWarningKeys" in view_model,
    "neutral_delta": '"neutral"' in view_model,
    "display_row_limit_6": "DISPLAY_ROW_LIMIT: usize = 6" in contract,
}
print(json.dumps(result, ensure_ascii=False, indent=2))
