from pathlib import Path
import argparse, re

parser=argparse.ArgumentParser()
parser.add_argument('--tree', choices=['baseline','modified'], required=True)
args=parser.parse_args()
artifact=Path(__file__).resolve().parent
root=artifact/'original' if args.tree=='baseline' else artifact/'modified'
settings=(root/'frontend/src/features/settings/settings-page.tsx').read_text(encoding='utf-8-sig')
hud=(root/'frontend/src/features/technical-hud/technical-hud-page.tsx').read_text(encoding='utf-8-sig')
window=(root/'src-tauri/src/commands/settings.rs').read_text(encoding='utf-8-sig')
resource=(root/'src/storage/resource.rs').read_text(encoding='utf-8-sig')
window_block=window.split('pub(crate) fn open_settings_hud_editor',1)[1].split('fn parse_hud_option',1)[0]
assignment_block=(resource.split('pub(crate) fn assign_missing_character_colors',1)[1].split('fn parse_character_hex_rgb',1)[0] if 'pub(crate) fn assign_missing_character_colors' in resource else '')
palette_block=(resource.split('const DISTINCT_CHARACTER_PALETTE',1)[1].split('= [',1)[1].split('];',1)[0] if 'const DISTINCT_CHARACTER_PALETTE' in resource else '')
print(f'TREE={args.tree}')
print(f'HUD_NATIVE_DRAG={str("draggable={!movePending}" in hud).lower()}')
print(f'SETTINGS_NATIVE_DRAG={str("draggable={!disabled}" in settings).lower()}')
print(f'SETTINGS_TWO_COLUMN={str("min-[720px]:grid-cols-2" in settings).lower()}')
print(f'SETTINGS_ONE_COLUMN={str("mt-3 flex flex-col overflow-hidden rounded-md border" in settings).lower()}')
print(f'SETTINGS_HIDES_MAIN={str("main_window.hide()" in window_block).lower()}')
print(f'AVATAR_PIXEL_COLOR_SOURCE={str("avatar_accent_rgb" in resource).lower()}')
print(f'DISTINCT_PALETTE_SIZE={len(re.findall(r"\[0x[0-9A-F]{2}, 0x[0-9A-F]{2}, 0x[0-9A-F]{2}\]",palette_block))}')
print(f'ASSIGNMENT_USES_ATTRIBUTE={str("attribute" in assignment_block).lower()}')

