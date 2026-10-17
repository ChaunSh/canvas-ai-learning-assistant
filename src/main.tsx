import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import "./styles/index.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("root element missing");
}

ReactDOM.createRoot(container).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
