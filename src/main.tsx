import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ConfigProvider } from "antd";
import "antd/dist/reset.css";
import App from "./App.tsx";
import { I18nProvider } from "./i18n/I18nProvider";
import { codexDeckAntdTheme } from "./theme/codexDeckTokens";
import "./index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ConfigProvider theme={codexDeckAntdTheme}>
      <I18nProvider>
        <App />
      </I18nProvider>
    </ConfigProvider>
  </StrictMode>,
);
