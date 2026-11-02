"use client";

import { Suspense } from "react";
import { MemoryPageClient } from "@/components/memory/memory";
import { SkeletonPage } from "@/components/pulse/panel";

export default function MemoryPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <MemoryPageClient />
    </Suspense>
  );
}
