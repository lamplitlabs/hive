"use client";

import { Suspense } from "react";
import { MemoryPageClient } from "@/components/memory/memory";
import { SkeletonPage } from "@/components/hive/panel";

export default function MemoryPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <MemoryPageClient />
    </Suspense>
  );
}
