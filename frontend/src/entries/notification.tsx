import { IslandPage } from "@/features/island/island-page";

import { renderWindow } from "./window-bootstrap";

renderWindow(<IslandPage />, {
  windowRoute: "notification-island",
  characterAvatars: false,
});
