import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { AddWindow } from "./AddWindow";
import { ProgressWindow } from "./progress/ProgressWindow";
import "./styles.css";

// Progress windows load the same page with "#progress/<id>".
const progress = /^#progress\/(\d+)$/.exec(window.location.hash);
const add = /^#add\/\d+$/.test(window.location.hash);

createRoot(document.getElementById("root")!).render(
  <StrictMode>{add ? <AddWindow /> : progress ? <ProgressWindow id={Number(progress[1])} /> : <App />}</StrictMode>,
);
