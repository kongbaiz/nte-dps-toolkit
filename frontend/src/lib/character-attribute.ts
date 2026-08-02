import attribute01 from "@res/images/attributes/UI_avatarbg_Icon_01.png";
import attribute02 from "@res/images/attributes/UI_avatarbg_Icon_02.png";
import attribute03 from "@res/images/attributes/UI_avatarbg_Icon_03.png";
import attribute04 from "@res/images/attributes/UI_avatarbg_Icon_04.png";
import attribute05 from "@res/images/attributes/UI_avatarbg_Icon_05.png";
import attribute06 from "@res/images/attributes/UI_avatarbg_Icon_06.png";

const attributeIcons: Readonly<Record<string, string>> = {
  灵: attribute01,
  相: attribute02,
  暗: attribute03,
  光: attribute04,
  魂: attribute05,
  咒: attribute06,
};

export function characterAttributeUrl(attribute: string | null): string | null {
  return attribute === null ? null : (attributeIcons[attribute] ?? null);
}
