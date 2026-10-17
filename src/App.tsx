import { useEffect } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "react-router-dom";

import { router } from "@/app/router";
import { Toaster } from "@/components/ui";
import { useSettings, watchSystemTheme } from "@/stores/settings";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Local SQLite is fast; a short stale window keeps screens consistent
      // without hammering the bridge.
      staleTime: 5_000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
});

export default function App() {
  const hydrate = useSettings((s) => s.hydrate);

  useEffect(() => {
    void hydrate();
    return watchSystemTheme();
  }, [hydrate]);

  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Toaster />
    </QueryClientProvider>
  );
}
