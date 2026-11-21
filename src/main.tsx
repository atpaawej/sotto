import "@fontsource-variable/instrument-sans";
import "@fontsource/instrument-serif/400.css";
import "./styles.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import Pill from "./Pill";

const root = document.getElementById("root");
if (!root) {
  throw new Error("the page has no #root element to mount into");
}

// One bundle, two windows: the pill is the same page with a query flag.
const showingPill = new URLSearchParams(window.location.search).has("window");

if (showingPill) {
  document.body.classList.add("pill-body");
}

createRoot(root).render(
  <StrictMode>{showingPill ? <Pill /> : <App />}</StrictMode>,
);
