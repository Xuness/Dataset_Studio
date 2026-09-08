const icon = new URL("./ds.svg", import.meta.url).href;
export function Brand({
  size = 28,
  className = "",
}: {
  size?: number;
  className?: string;
}) {
  return (
    <img
      className={"brand-image " + className}
      src={icon}
      width={size}
      height={size}
      alt="Dataset Studio"
      draggable={false}
    />
  );
}
