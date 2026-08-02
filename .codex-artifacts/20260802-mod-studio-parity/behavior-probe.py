from pathlib import Path
import sys
artifact=Path(sys.argv[1])

def read(base, rel): return (base/rel).read_text(encoding='utf-8')
def probe(name, base):
    page=read(base,'frontend/src/features/mod-studio/mod-studio-page.tsx')
    commands=read(base,'src-tauri/src/commands/mod_studio.rs')
    platform=read(base,'src/platform/mods_plugin.rs')
    core=read(base,'src/core/mod_studio.rs')
    values={
      'fixed_region_disabled': 'defaultValue="cn"' in page and 'defaultValue="cn"\n          disabled' in page,
      'open_folder_action': 'onClick={() => void onOpenFolder()}' in page,
      'create_document_action': 'create_mod_studio_document' in commands and 'create_mod_studio_document' in core,
      'manual_directory_action': 'choose_mod_studio_game_directory' in commands and 'resolve_manual_game_directory' in platform,
      'china_global_options': '<option value="china">' in page and '<option value="global">' in page,
      'loader_action': 'set_mod_studio_loader_enabled' in commands,
    }
    print(f'[{name}]')
    for key,value in values.items(): print(f'{key}={str(value).lower()}')
    return values
baseline=probe('baseline',artifact/'original')
modified=probe('modified',artifact/'modified-snapshot')
expected_baseline={'fixed_region_disabled':True,'open_folder_action':False,'create_document_action':False,'manual_directory_action':False,'china_global_options':False,'loader_action':False}
expected_modified={'fixed_region_disabled':False,'open_folder_action':True,'create_document_action':True,'manual_directory_action':True,'china_global_options':True,'loader_action':True}
ok=baseline==expected_baseline and modified==expected_modified
print(f'behavior_probe_passed={str(ok).lower()}')
raise SystemExit(0 if ok else 1)
