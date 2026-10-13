"use client"

import * as React from "react"
import { cn } from "cn"
import { Progress as ProgressPrimitive } from "radix-ui"

function Progress({
  className,
  value,
  max = 100,
  ...props
}: React.ComponentProps<typeof ProgressPrimitive.Root>) {
  const limit = Number.isFinite(max) && max > 0 ? max : 100
  const progress = typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= limit ? value : null
  return (
    <ProgressPrimitive.Root
      value={progress}
      max={limit}
      data-slot="progress"
      className={cn(
        "relative flex h-3 w-full items-center overflow-x-hidden rounded-4xl bg-muted",
        className
      )}
      {...props}
    >
      <ProgressPrimitive.Indicator
        data-slot="progress-indicator"
        className={cn("h-full bg-primary transition-all", progress === null ? "w-1/3 animate-pulse motion-reduce:animate-none" : "w-full")}
        style={progress === null ? undefined : { transform: `translateX(-${100 - progress / limit * 100}%)` }}
      />
    </ProgressPrimitive.Root>
  )
}

export { Progress }
