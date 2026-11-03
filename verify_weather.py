import asyncio
from weather import get_forecast

async def test_weather():
    print("Testing get_forecast for New York (40.7128, -74.0060)...")
    try:
        # FastMCP tools are often callable directly if designed that way, 
        # or we might need to access the underlying function.
        # Let's try calling it directly first.
        result = await get_forecast(40.7128, -74.0060)
        print("Result:")
        print(result)
    except Exception as e:
        print(f"Error calling tool: {e}")

if __name__ == "__main__":
    asyncio.run(test_weather())
