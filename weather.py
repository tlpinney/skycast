from typing import Any
import httpx
from mcp.server.fastmcp import FastMCP

# Initialize FastMCP server
mcp = FastMCP("weather")

@mcp.tool()
async def get_forecast(latitude: float | None = None, longitude: float | None = None, location_name: str | None = None) -> str:
    """Get the current weather forecast for a specific location.
    
    Args:
        latitude: The latitude of the location (optional if location_name provided).
        longitude: The longitude of the location (optional if location_name provided).
        location_name: The name of the location (e.g., "New York", "Herndon, VA").
    """
    if location_name:
        try:
            lat, lon = await geocode(location_name)
            latitude = lat
            longitude = lon
        except ValueError as e:
            return str(e)
    
    if latitude is None or longitude is None:
        return "Error: You must provide either (latitude, longitude) or location_name."

    url = "https://api.open-meteo.com/v1/forecast"
    params = {
        "latitude": latitude,
        "longitude": longitude,
        "current": ["temperature_2m", "wind_speed_10m", "weather_code"],
        "hourly": ["temperature_2m", "precipitation_probability", "weather_code"],
        "forecast_days": 1
    }

    async with httpx.AsyncClient() as client:
        try:
            response = await client.get(url, params=params)
            response.raise_for_status()
            data = response.json()
            
            current = data.get("current", {})
            units = data.get("current_units", {})
            
            location_str = f"{location_name} ({latitude}, {longitude})" if location_name else f"({latitude}, {longitude})"
            
            # Format a nice summary
            summary = (
                f"Weather for {location_str}:\n"
                f"Temperature: {current.get('temperature_2m')} {units.get('temperature_2m')}\n"
                f"Wind Speed: {current.get('wind_speed_10m')} {units.get('wind_speed_10m')}\n"
                f"Weather Code: {current.get('weather_code')} (WMO code)"
            )
            return summary
        except Exception as e:
            return f"Error fetching weather data: {str(e)}"

async def geocode(location_name: str) -> tuple[float, float]:
    """Geocode a location name to latitude and longitude."""
    url = "https://geocoding-api.open-meteo.com/v1/search"
    params = {
        "name": location_name,
        "count": 1,
        "language": "en",
        "format": "json"
    }
    
    async with httpx.AsyncClient() as client:
        response = await client.get(url, params=params)
        response.raise_for_status()
        data = response.json()
        
        if not data.get("results"):
            raise ValueError(f"Location '{location_name}' not found.")
            
        result = data["results"][0]
        return result["latitude"], result["longitude"]

if __name__ == "__main__":
    mcp.run()
