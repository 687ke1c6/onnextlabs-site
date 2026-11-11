/* @refresh reload */
import { render } from "solid-js/web";
import { RouterProvider } from "@tanstack/solid-router";
import "./index.css";
import { router } from "./router";

render(() => <RouterProvider router={router} />, document.getElementById("root")!);
