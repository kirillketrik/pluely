import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

let cached: boolean | null = null;

/**
 * True when this build was compiled with the Pluely server endpoint.
 * Forks without it must not show or call the server-backed features
 * (Pluely models, prompts, usage), which would only produce errors.
 */
export const usePluelyBackend = (): boolean | null => {
  const [available, setAvailable] = useState<boolean | null>(cached);

  useEffect(() => {
    if (cached !== null) return;
    invoke<boolean>("pluely_backend_available")
      .then((value) => {
        cached = value;
        setAvailable(value);
      })
      .catch(() => {
        cached = false;
        setAvailable(false);
      });
  }, []);

  return available;
};
