"""Export only referenced panel artwork; never export account/build/stat values."""
from pathlib import Path
import hashlib
import json
import shutil
import struct

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'NTE_Assets/CN/HT/Content'

def export():
    sources = {}
    def rows(relative):
        path = SOURCE / relative
        data = path.read_bytes()
        sources[relative] = hashlib.sha256(data).hexdigest()
        return json.loads(data)[0]['Rows']
    def image(ref, folder):
        asset = ref['AssetPathName'].split('.')[0]
        if not asset.startswith('/Game/'):
            return None
        path = SOURCE / (asset.removeprefix('/Game/') + '.png')
        if not path.is_file():
            return None
        data = path.read_bytes()
        if data[:8] != b'\x89PNG\r\n\x1a\n':
            raise ValueError('Invalid PNG: ' + path.name)
        width, height = struct.unpack('>II', data[16:24])
        if not (0 < width <= 4096 and 0 < height <= 4096):
            raise ValueError('Unexpected dimensions: ' + path.name)
        target = ROOT / 'res/images/character-panel' / folder / path.name
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.exists() or target.read_bytes() != data:
            shutil.copyfile(path, target)
        assert target.read_bytes() == data
        return target.relative_to(ROOT).as_posix()
    playable = json.loads((ROOT / 'res/data/characters/characters.json').read_bytes())['characters']
    characters = {key: image(row['CharacterTabImg'], 'portraits') for key, row in rows('DataTable/Equipment/DT_EquipmentPlanData.json').items() if key in playable}
    arcs = {key: image(row['ItemIcon'], 'arcs') for key, row in rows('DataTable/Fork/DT_ForkItemData.json').items()}
    result = {'sources': sources, 'characters': characters, 'arcs': arcs}
    target = ROOT / 'res/data/characters/panel_art.json'
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    assert json.loads(target.read_bytes()) == result
    print(f'Panel art: {sum(v is not None for v in characters.values())} portraits, {sum(v is not None for v in arcs.values())} arcs; missing: {[k for k,v in {**characters,**arcs}.items() if v is None]}')

if __name__ == '__main__':
    export()
