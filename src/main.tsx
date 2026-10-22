import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";
import "./themes.css";

const root = document.getElementById("root");
if (!root) throw new Error("The Folio application root is missing.");
createRoot(root).render(<App />);
