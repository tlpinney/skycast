import asyncio
from weather import get_forecast

async def test_geocoding():
    print("Testing get_forecast for 'Herndon, VA'...")
    try:
        result = await get_forecast(location_name="Herndon, VA")
        print("Result:")
        print(result)
    except Exception as e:
        print(f"Error calling tool: {e}")

    print("\nTesting get_forecast for 'Paris'...")
    try:
        result = await get_forecast(location_name="Paris")
        print("Result:")
        print(result)
    except Exception as e:
        print(f"Error calling tool: {e}")

if __name__ == "__main__":
    asyncio.run(test_geocoding())
