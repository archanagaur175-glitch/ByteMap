// Category + tier presentation (PROJECT_BRIEF token table). Colors are
// keyed by the wire ids shared with the Rust categorizer.

export const CATEGORY_COLORS: Record<number, string> = {
  0: "#4FD1C5", // Code
  1: "#A78BFA", // System
  2: "#FFB347", // Archives
  3: "#FF5C8A", // Media
  4: "#5B9DFF", // Documents
  5: "#8BDD6B", // Apps
  6: "#8892A6", // Other
};

export const CATEGORY_NAMES: Record<number, string> = {
  0: "Code",
  1: "System",
  2: "Archives",
  3: "Media",
  4: "Documents",
  5: "Apps",
  6: "Other",
};

export const TIER_COLORS: Record<number, string> = {
  0: "#3DDC97", // Safe
  1: "#FFC857", // Review
  2: "#FF5D5D", // Protected
};

export const TIER_NAMES: Record<number, string> = {
  0: "Safe",
  1: "Review",
  2: "Protected",
};
