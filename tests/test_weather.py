import pytest
import respx
from httpx import Response
from weather import get_forecast

@pytest.mark.asyncio
async def test_get_forecast_lat_lon():
    async with respx.mock:
        # Mock the weather API response
        respx.get("https://api.open-meteo.com/v1/forecast").mock(
            return_value=Response(
                200,
                json={
                    "current": {
                        "temperature_2m": 20.5,
                        "wind_speed_10m": 10.0,
                        "weather_code": 1
                    },
                    "current_units": {
                        "temperature_2m": "°C",
                        "wind_speed_10m": "km/h"
                    }
                }
            )
        )

        result = await get_forecast(latitude=40.7128, longitude=-74.0060)
        
        assert "Temperature: 20.5 °C" in result
        assert "Wind Speed: 10.0 km/h" in result
        assert "Weather Code: 1" in result

@pytest.mark.asyncio
async def test_get_forecast_location_name():
    async with respx.mock:
        # Mock the geocoding API response
        respx.get("https://geocoding-api.open-meteo.com/v1/search").mock(
            return_value=Response(
                200,
                json={
                    "results": [
                        {"latitude": 51.5074, "longitude": -0.1278, "name": "London"}
                    ]
                }
            )
        )
        
        # Mock the weather API response
        respx.get("https://api.open-meteo.com/v1/forecast").mock(
            return_value=Response(
                200,
                json={
                    "current": {
                        "temperature_2m": 15.0,
                        "wind_speed_10m": 5.0,
                        "weather_code": 2
                    },
                    "current_units": {
                        "temperature_2m": "°C",
                        "wind_speed_10m": "km/h"
                    }
                }
            )
        )

        result = await get_forecast(location_name="London")
        
        assert "Weather for London" in result
        assert "Temperature: 15.0 °C" in result

@pytest.mark.asyncio
async def test_get_forecast_invalid_location():
    async with respx.mock:
        # Mock empty geocoding response
        respx.get("https://geocoding-api.open-meteo.com/v1/search").mock(
            return_value=Response(200, json={"results": []})
        )

        result = await get_forecast(location_name="NonExistentPlace")
        
        assert "Location 'NonExistentPlace' not found" in result

@pytest.mark.asyncio
async def test_get_forecast_missing_args():
    result = await get_forecast()
    assert "Error: You must provide either" in result

@pytest.mark.asyncio
async def test_api_error():
    async with respx.mock:
        respx.get("https://api.open-meteo.com/v1/forecast").mock(
            return_value=Response(500)
        )

        result = await get_forecast(latitude=0, longitude=0)
        assert "Error fetching weather data" in result
