import { mark } from "./perf";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/fonts.css";
import "./styles/design.css";
import "./styles/screens.css";
import "./styles/stages.css";
import "./styles/ink.css";

mark("js:render");
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
