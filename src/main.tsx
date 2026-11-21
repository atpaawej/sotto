import "@fontsource-variable/instrument-sans";
import "@fontsource/instrument-serif/400.css";
import "./styles.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

const root = document.getElementById("root");
if (!root) {
  throw new Error("the page has no #root element to mount into");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
