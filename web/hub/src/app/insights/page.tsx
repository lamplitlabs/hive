"use client";

import { Suspense } from "react";
import { Insights } from "@/components/insights/insights";
import { SkeletonPage } from "@/components/pulse/panel";

// /insights/: the over-time metrics that used to close the Home page. The
// range and project filters live in the search params (hrefs.insights), so
// the screen mounts under Suspense like the other static-export pages.
export default function InsightsPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={4} />}>
      <Insights />
    </Suspense>
  );
}
