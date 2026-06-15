import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Кореневий елемент #app не знайдено");
}

const app = mount(App, { target });

export default app;
