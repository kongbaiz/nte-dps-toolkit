export interface DamageImageLookup {
  digit(prefix: string, digit: string): string | undefined;
  reaction(folder: string, reaction: number): readonly string[];
}

const REACTION_IMAGE_PARTS = ["01", "02"] as const;

export function createDamageImageLookup(
  damageDigitImages: Readonly<Record<string, string>>,
  reactionLabelImages: Readonly<Record<string, string>>,
): DamageImageLookup {
  const digitUrls = indexImageModules(damageDigitImages, imageFileStem);
  const reactionUrls = indexImageModules(
    reactionLabelImages,
    localizedImageKey,
  );

  return {
    digit(prefix, digit) {
      return digitUrls.get(`${prefix}_${digit}`);
    },
    reaction(folder, reaction) {
      const stem = `fanying${String(reaction).padStart(2, "0")}`;
      const urls: string[] = [];
      for (const part of REACTION_IMAGE_PARTS) {
        const url = reactionUrls.get(`${folder}/${stem}_${part}`);
        if (url !== undefined) urls.push(url);
      }
      return urls;
    },
  };
}

function indexImageModules(
  images: Readonly<Record<string, string>>,
  keyForPath: (path: string) => string | null,
): ReadonlyMap<string, string> {
  const urls = new Map<string, string>();
  for (const [path, url] of Object.entries(images)) {
    const key = keyForPath(path);
    if (key !== null) urls.set(key, url);
  }
  return urls;
}

function imageFileStem(path: string): string | null {
  const normalized = path.replaceAll("\\", "/");
  const fileName = normalized.slice(normalized.lastIndexOf("/") + 1);
  return fileName.endsWith(".png") ? fileName.slice(0, -4) : null;
}

function localizedImageKey(path: string): string | null {
  const normalized = path.replaceAll("\\", "/");
  const match = normalized.match(/\/(zh|en|ja)\/([^/]+)\.png$/);
  return match === null ? null : `${match[1]}/${match[2]}`;
}
