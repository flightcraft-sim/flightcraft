import init from "./flightcraft_web.js";

init().catch((error) => {
  console.error("FlightCraft could not load:", error);
  const loading = document.getElementById("loading");
  if (loading) loading.textContent = `FlightCraft could not load: ${error}`;
});
