// SPDX-License-Identifier: Apache-2.0

import { createFromSource } from "fumadocs-core/search/server";
import { source } from "@/lib/source";

// Built once into a static index; the search dialog loads it in the browser.
export const revalidate = false;
export const { staticGET: GET } = createFromSource(source, { language: "english" });
