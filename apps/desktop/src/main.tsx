import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { tauriBridge } from "./bridge";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("Kesharon root element is missing");
}

createRoot(root).render(
  <StrictMode>
    <App bridge={tauriBridge} />
  </StrictMode>
);
