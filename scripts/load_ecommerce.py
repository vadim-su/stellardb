#!/usr/bin/env python3
"""Load realistic e-commerce test data into StellarDB."""

import hashlib
import math
import random
import string
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timedelta

import requests

BASE_URL = "http://127.0.0.1:3000"
DATABASE = "ecommerce"
WORKERS = 16

# Data volumes
NUM_CUSTOMER = 10_000
NUM_PRODUCT = 5_000
NUM_ORDER = 100_000
NUM_REVIEW = 50_000

BATCH_SIZE = 100


# ============================================================
# Reference data
# ============================================================

CITIES = [
    ("Moscow", "Russia"),
    ("Saint Petersburg", "Russia"),
    ("Berlin", "Germany"),
    ("Munich", "Germany"),
    ("Hamburg", "Germany"),
    ("Paris", "France"),
    ("Lyon", "France"),
    ("Marseille", "France"),
    ("London", "UK"),
    ("Manchester", "UK"),
    ("Birmingham", "UK"),
    ("New York", "USA"),
    ("Los Angeles", "USA"),
    ("Chicago", "USA"),
    ("Houston", "USA"),
    ("Tokyo", "Japan"),
    ("Osaka", "Japan"),
    ("Kyoto", "Japan"),
    ("Beijing", "China"),
    ("Shanghai", "China"),
    ("Shenzhen", "China"),
    ("Sydney", "Australia"),
    ("Melbourne", "Australia"),
    ("Toronto", "Canada"),
    ("Vancouver", "Canada"),
    ("Amsterdam", "Netherlands"),
    ("Rotterdam", "Netherlands"),
    ("Madrid", "Spain"),
    ("Barcelona", "Spain"),
    ("Rome", "Italy"),
    ("Milan", "Italy"),
]

TIERS = ["bronze", "silver", "gold", "platinum"]
TIER_WEIGHTS = [50, 30, 15, 5]

TAGS = [
    "newsletter",
    "early_adopter",
    "vip",
    "wholesale",
    "retail",
    "verified",
    "inactive",
    "promo_sensitive",
    "high_value",
    "new",
]

CATEGORIES = {
    "Electronics": ["Smartphones", "Laptops", "Tablets", "Headphones", "Cameras", "TVs", "Speakers"],
    "Clothing": ["T-Shirts", "Jeans", "Dresses", "Jackets", "Shoes", "Accessories"],
    "Home": ["Furniture", "Kitchen", "Bedding", "Lighting", "Decor", "Storage"],
    "Sports": ["Fitness", "Outdoor", "Team Sports", "Water Sports", "Winter Sports"],
    "Books": ["Fiction", "Non-Fiction", "Technical", "Children", "Comics"],
    "Beauty": ["Skincare", "Makeup", "Haircare", "Fragrance", "Tools"],
    "Food": ["Snacks", "Beverages", "Organic", "Supplements", "Gourmet"],
    "Toys": ["Board Games", "Action Figures", "Educational", "Outdoor Toys", "Puzzles"],
}

BRANDS = [
    "TechPro",
    "StyleMax",
    "HomeComfort",
    "SportX",
    "ReadMore",
    "GlowUp",
    "TasteBest",
    "PlayFun",
    "ValueBrand",
    "PremiumChoice",
    "EcoLine",
    "FastTrack",
    "QualityFirst",
    "BudgetSmart",
    "LuxeLife",
]

ORDER_STATUSES = ["pending", "paid", "shipped", "delivered", "cancelled"]
STATUS_WEIGHTS = [5, 10, 15, 60, 10]

FIRST_NAMES = [
    "Alexander",
    "Maria",
    "Ivan",
    "Anna",
    "Dmitry",
    "Elena",
    "Sergey",
    "Olga",
    "John",
    "Emma",
    "Michael",
    "Sophie",
    "James",
    "Olivia",
    "William",
    "Isabella",
    "Hans",
    "Greta",
    "Pierre",
    "Marie",
    "Carlos",
    "Carmen",
    "Yuki",
    "Sakura",
    "Wei",
    "Mei",
    "Ahmed",
    "Fatima",
    "Raj",
    "Priya",
    "Lucas",
    "Mia",
]

LAST_NAMES = [
    "Ivanov",
    "Petrov",
    "Sidorov",
    "Smith",
    "Johnson",
    "Williams",
    "Brown",
    "Mueller",
    "Schmidt",
    "Dubois",
    "Martin",
    "Garcia",
    "Rodriguez",
    "Tanaka",
    "Yamamoto",
    "Wang",
    "Li",
    "Chen",
    "Kim",
    "Park",
    "Singh",
    "Patel",
]

REVIEW_TITLES_POSITIVE = [
    "Great product!",
    "Exceeded expectations",
    "Highly recommend",
    "Best purchase ever",
    "Amazing quality",
    "Worth every penny",
    "Five stars!",
    "Love it",
    "Perfect",
    "Fantastic",
    "Absolutely brilliant",
    "Game changer",
    "Must have item",
    "Outstanding value",
    "Superb quality",
    "Couldn't be happier",
    "Top notch",
    "Impressive",
    "Exactly as advertised",
    "Will buy again",
]

REVIEW_TITLES_NEGATIVE = [
    "Disappointed",
    "Not as described",
    "Poor quality",
    "Would not recommend",
    "Waste of money",
    "Broke after a week",
    "Terrible",
    "Avoid",
    "Regret buying",
    "Not worth it",
    "Complete failure",
    "Save your money",
    "Very frustrating",
    "Defective product",
    "Total letdown",
    "Cheaply made",
    "False advertising",
    "Horrible experience",
    "Stay away",
    "One star is generous",
]

REVIEW_TITLES_NEUTRAL = [
    "It's okay",
    "Average product",
    "Does the job",
    "Nothing special",
    "As expected",
    "Decent",
    "Mixed feelings",
    "Good enough",
    "Fair value",
    "Middle of the road",
    "Acceptable quality",
    "Works fine",
    "Could be better",
    "Not bad",
    "Meets basic needs",
]

# Expanded review text components for more variety
REVIEW_INTRO_POSITIVE = [
    "I'm really impressed with this purchase.",
    "This exceeded all my expectations.",
    "Absolutely love this product!",
    "One of the best purchases I've made.",
    "I can't say enough good things about this.",
    "This is exactly what I needed.",
    "So glad I decided to buy this.",
    "This product is a game changer.",
    "I was pleasantly surprised by the quality.",
    "From the moment I opened it, I knew it was special.",
]

REVIEW_QUALITY_POSITIVE = [
    "The quality is exceptional.",
    "Build quality is outstanding.",
    "Premium materials throughout.",
    "Feels solid and well-made.",
    "Craftsmanship is top-notch.",
    "You can tell this is made to last.",
    "The attention to detail is impressive.",
    "Superior construction quality.",
    "Excellent fit and finish.",
    "Materials feel luxurious.",
]

REVIEW_VALUE_POSITIVE = [
    "Worth every penny I spent.",
    "Great value for the money.",
    "You get what you pay for and more.",
    "An excellent investment.",
    "Priced fairly for the quality.",
    "Better than products twice the price.",
    "Incredible bang for your buck.",
    "The price point is perfect.",
    "Would have paid more for this quality.",
    "A steal at this price.",
]

REVIEW_RECOMMEND_POSITIVE = [
    "I highly recommend this to everyone.",
    "Would definitely recommend to friends and family.",
    "If you're on the fence, just buy it.",
    "I've already told all my friends about this.",
    "Strongly recommend without hesitation.",
    "You won't regret this purchase.",
    "A must-have product.",
    "Go ahead and buy it, you won't be disappointed.",
    "I recommend this wholeheartedly.",
    "This deserves all the five-star reviews.",
]

REVIEW_CLOSING_POSITIVE = [
    "Will definitely buy again.",
    "Already planning to purchase more.",
    "This has become my go-to choice.",
    "Couldn't be happier with my decision.",
    "A perfect addition to my collection.",
    "Exactly as described, if not better.",
    "Fast shipping and great packaging too.",
    "Customer service was also excellent.",
    "This company has earned my loyalty.",
    "Looking forward to trying other products from this brand.",
]

REVIEW_INTRO_NEGATIVE = [
    "Very disappointed with this purchase.",
    "This was a complete waste of money.",
    "I expected much better quality.",
    "Regret buying this product.",
    "Save yourself the trouble and avoid this.",
    "This is not what I ordered.",
    "I'm extremely frustrated with this product.",
    "Biggest shopping mistake I've made.",
    "This product is a total failure.",
    "I want my money back.",
]

REVIEW_QUALITY_NEGATIVE = [
    "The quality is absolutely terrible.",
    "Cheaply made garbage.",
    "Feels flimsy and poorly constructed.",
    "Materials are low quality.",
    "Build quality is non-existent.",
    "It broke within days of arrival.",
    "The craftsmanship is awful.",
    "Looks nothing like the pictures.",
    "Clearly cut corners everywhere.",
    "Feels like a cheap knockoff.",
]

REVIEW_VALUE_NEGATIVE = [
    "Not worth a fraction of the price.",
    "Complete rip-off.",
    "Overpriced for what you get.",
    "Should cost half as much.",
    "A waste of hard-earned money.",
    "You're paying for marketing, not quality.",
    "Better options exist for less money.",
    "Terrible value proposition.",
    "Feel cheated by this purchase.",
    "Money down the drain.",
]

REVIEW_WARNING_NEGATIVE = [
    "Would not recommend to anyone.",
    "Stay away from this product.",
    "Don't make the same mistake I did.",
    "Avoid this at all costs.",
    "I cannot recommend this in good conscience.",
    "Do yourself a favor and look elsewhere.",
    "Warning: do not buy this.",
    "I'm telling everyone to avoid this brand.",
    "Please read negative reviews before buying.",
    "Wish I had seen the bad reviews first.",
]

REVIEW_CLOSING_NEGATIVE = [
    "Returning this immediately.",
    "Already requested a refund.",
    "Never buying from this brand again.",
    "Customer service was unhelpful too.",
    "The return process was also a nightmare.",
    "Lesson learned the hard way.",
    "Throwing this in the trash.",
    "Will be disputing the charge.",
    "Total disappointment from start to finish.",
    "One of my worst online purchases ever.",
]

REVIEW_INTRO_NEUTRAL = [
    "This product is okay, nothing special.",
    "It's fine for what it is.",
    "Met my basic expectations.",
    "Neither great nor terrible.",
    "A middle-of-the-road product.",
    "It does what it's supposed to do.",
    "Acceptable but unremarkable.",
    "Not the best, not the worst.",
    "It's a decent option.",
    "Fair enough for the price.",
]

REVIEW_QUALITY_NEUTRAL = [
    "Quality is average at best.",
    "Build quality could be better.",
    "Adequate construction.",
    "Nothing impressive about the materials.",
    "It's serviceable quality.",
    "Feels okay, nothing premium.",
    "Standard quality for this price range.",
    "Could use some improvements.",
    "The quality matches the price.",
    "Passable but not exceptional.",
]

REVIEW_VALUE_NEUTRAL = [
    "You get what you pay for.",
    "Fair value, I suppose.",
    "Not a great deal, not a rip-off.",
    "Price is about right.",
    "Could be priced a bit lower.",
    "Acceptable for the cost.",
    "Neither overpriced nor a bargain.",
    "The price matches the quality.",
    "Worth considering if on sale.",
    "Reasonable for what it offers.",
]

REVIEW_CLOSING_NEUTRAL = [
    "Might consider buying again if needed.",
    "It serves its purpose.",
    "No strong feelings either way.",
    "Works for now.",
    "Will see how it holds up over time.",
    "Not sure if I'd recommend it.",
    "There are probably better options out there.",
    "It's functional, at least.",
    "Does the job, barely.",
    "Mixed feelings overall.",
]

# Product descriptions for FTS - expanded for variety
PRODUCT_DESCRIPTIONS = {
    "Electronics": [
        "High-performance device with cutting-edge technology and sleek design.",
        "Premium quality electronics built for everyday use and durability.",
        "Advanced features combined with user-friendly interface.",
        "Innovative technology that delivers exceptional performance.",
        "State-of-the-art electronics with smart connectivity options.",
        "Next-generation device featuring breakthrough engineering.",
        "Compact yet powerful electronics for modern lifestyles.",
        "Industry-leading specifications at an unbeatable price point.",
        "Seamlessly integrates with your existing tech ecosystem.",
        "Award-winning design meets powerful functionality.",
        "Engineered for speed, reliability, and energy efficiency.",
        "Crystal-clear display with vibrant colors and sharp details.",
        "Long-lasting battery life keeps you connected all day.",
        "Intuitive controls make operation effortless for all users.",
        "Built with sustainable materials for eco-conscious consumers.",
    ],
    "Clothing": [
        "Comfortable and stylish apparel made from premium materials.",
        "Fashion-forward design with excellent craftsmanship.",
        "Versatile piece perfect for any occasion and season.",
        "High-quality fabric that ensures long-lasting wear.",
        "Modern style meets classic comfort in this essential piece.",
        "Breathable fabric keeps you cool and comfortable all day.",
        "Tailored fit that flatters every body type beautifully.",
        "Machine washable and maintains shape after multiple washes.",
        "Trendy design inspired by the latest runway collections.",
        "Sustainable fashion made from recycled materials.",
        "Wrinkle-resistant fabric perfect for travel and busy days.",
        "Soft touch fabric feels luxurious against your skin.",
        "Bold patterns that make a statement wherever you go.",
        "Timeless classic that never goes out of style.",
        "Lightweight layers perfect for transitional weather.",
    ],
    "Home": [
        "Beautiful home decor that adds elegance to any room.",
        "Functional design combined with aesthetic appeal.",
        "Quality craftsmanship for your living space.",
        "Modern home essential with timeless style.",
        "Durable and practical solution for everyday living.",
        "Handcrafted piece with artisanal attention to detail.",
        "Space-saving design perfect for smaller apartments.",
        "Easy assembly with all tools and hardware included.",
        "Multi-functional furniture that adapts to your needs.",
        "Stain-resistant surface makes cleaning a breeze.",
        "Eco-friendly materials sourced responsibly.",
        "Minimalist aesthetic complements any interior style.",
        "Sturdy construction supports heavy daily use.",
        "Neutral tones blend seamlessly with existing decor.",
        "Smart storage solutions maximize your living space.",
    ],
    "Sports": [
        "Professional-grade equipment for athletes and enthusiasts.",
        "Engineered for performance and built to last.",
        "Lightweight design with maximum durability.",
        "Perfect for training, competition, and recreational use.",
        "Advanced materials for superior comfort and performance.",
        "Ergonomic design reduces strain during extended use.",
        "Weather-resistant construction for outdoor adventures.",
        "Adjustable features customize fit for any user.",
        "High-visibility colors ensure safety during activities.",
        "Quick-dry technology keeps you comfortable during workouts.",
        "Shock-absorbing technology protects joints and muscles.",
        "Competition-tested and approved by professionals.",
        "Compact design folds flat for easy storage and transport.",
        "Non-slip grip provides secure handling in all conditions.",
        "Breathable mesh panels enhance ventilation and airflow.",
    ],
    "Books": [
        "Engaging read that captivates from start to finish.",
        "Well-researched content with insightful perspectives.",
        "A must-have addition to any book collection.",
        "Thought-provoking material for curious minds.",
        "Beautifully written with compelling storytelling.",
        "Award-winning author delivers another masterpiece.",
        "Page-turner that keeps you reading late into the night.",
        "Includes helpful illustrations and diagrams throughout.",
        "Perfect for book clubs and group discussions.",
        "Hardcover edition with premium binding and paper quality.",
        "Expert analysis backed by extensive research.",
        "Accessible writing style suitable for all reading levels.",
        "Includes exclusive bonus content and author notes.",
        "International bestseller translated into multiple languages.",
        "Timeless wisdom applicable to modern challenges.",
    ],
    "Beauty": [
        "Premium skincare formula with natural ingredients.",
        "Luxurious beauty product for radiant results.",
        "Gentle yet effective formula for all skin types.",
        "Professional-quality beauty essential.",
        "Nourishing ingredients for healthy, glowing skin.",
        "Dermatologist-tested and hypoallergenic formula.",
        "Long-lasting coverage that stays fresh all day.",
        "Cruelty-free and vegan-friendly formulation.",
        "Anti-aging properties reduce fine lines and wrinkles.",
        "Hydrating complex locks in moisture for hours.",
        "Lightweight texture absorbs quickly without residue.",
        "Clinically proven to improve skin texture in weeks.",
        "Paraben-free and free from harmful chemicals.",
        "Buildable formula lets you customize your look.",
        "Includes SPF protection for daily sun defense.",
    ],
    "Food": [
        "Delicious gourmet selection made with finest ingredients.",
        "Healthy and nutritious choice for mindful eating.",
        "Premium quality food product with authentic taste.",
        "Carefully sourced ingredients for superior flavor.",
        "Wholesome goodness in every bite.",
        "Organic certified and non-GMO verified product.",
        "Low-calorie option without sacrificing taste.",
        "Rich in protein and essential nutrients.",
        "Artisanal recipe passed down through generations.",
        "No artificial preservatives or additives included.",
        "Perfect balance of sweet and savory flavors.",
        "Gluten-free alternative for dietary restrictions.",
        "Sustainably harvested from responsible sources.",
        "Ready in minutes for quick and easy meals.",
        "Family-size portion great for sharing with loved ones.",
    ],
    "Toys": [
        "Fun and educational toy for hours of entertainment.",
        "Safe and durable playtime essential for kids.",
        "Encourages creativity and imaginative play.",
        "High-quality toy built to withstand active play.",
        "Perfect gift for children of all ages.",
        "STEM-focused learning disguised as playtime fun.",
        "Bright colors and engaging textures stimulate senses.",
        "Battery-free design promotes independent play.",
        "Includes multiple pieces for endless combinations.",
        "Award-winning educational toy recommended by teachers.",
        "Non-toxic materials safe for young children.",
        "Promotes fine motor skills and hand-eye coordination.",
        "Interactive features respond to touch and sound.",
        "Grows with your child through multiple learning stages.",
        "Compact storage container included for easy cleanup.",
    ],
}


# ============================================================
# Helpers
# ============================================================


def random_string(length=8):
    return "".join(random.choices(string.ascii_lowercase, k=length))


def random_date(start_year=2022, end_year=2024):
    """Generate a random date as datetime literal: d"2024-01-15T00:00:00Z" """
    start = datetime(start_year, 1, 1)
    end = datetime(end_year, 12, 31)
    delta = end - start
    random_days = random.randint(0, delta.days)
    dt = start + timedelta(days=random_days)
    return f'd"{dt.strftime("%Y-%m-%dT00:00:00Z")}"'


def random_datetime(start_year=2022, end_year=2024):
    """Generate a random datetime as datetime literal: d"2024-01-15T10:30:00Z" """
    start = datetime(start_year, 1, 1)
    end = datetime(end_year, 12, 31)
    delta = end - start
    random_seconds = random.randint(0, int(delta.total_seconds()))
    dt = start + timedelta(seconds=random_seconds)
    return f'd"{dt.strftime("%Y-%m-%dT%H:%M:%SZ")}"'


def random_duration():
    """Generate a random duration literal: 1h30m, 2d, 500ms, etc."""
    units = [
        ("ms", 1, 999),
        ("s", 1, 59),
        ("m", 1, 59),
        ("h", 1, 23),
        ("d", 1, 30),
    ]
    # Pick 1-2 units
    num_units = random.randint(1, 2)
    selected = random.sample(units, num_units)
    selected.sort(key=lambda x: ["d", "h", "m", "s", "ms"].index(x[0]))
    parts = [f"{random.randint(u[1], u[2])}{u[0]}" for u in selected]
    return "".join(parts)


def escape_string(s):
    """Escape double quotes for StellarQL strings."""
    return s.replace('"', '\\"')


def generate_embedding(text: str, dimension: int = 384) -> list:
    """
    Generate a deterministic fake embedding from text.
    Uses hash of text to seed random for reproducibility.
    This simulates what a real embedding model would produce.
    """
    # Use hash of text to seed random for reproducibility
    seed = int(hashlib.md5(text.encode()).hexdigest()[:8], 16)
    rng = random.Random(seed)
    # Generate normalized vector
    vec = [rng.gauss(0, 1) for _ in range(dimension)]
    # Normalize to unit vector for cosine similarity
    norm = math.sqrt(sum(x * x for x in vec))
    return [round(x / norm, 6) for x in vec]


def exec_query(query, timeout=30):
    """Execute a query and return success status."""
    try:
        r = requests.post(
            f"{BASE_URL}/sql",
            json={"query": query},
            headers={"X-Database": DATABASE},
            timeout=timeout,
        )
        if r.status_code != 200:
            return False, r.text[:200]
        body = r.json()
        if body.get("error"):
            return False, str(body["error"])[:200]
        return True, None
    except Exception as e:
        return False, str(e)[:200]


# ============================================================
# Data generators
# ============================================================


def generate_customer(customer_id):
    """Generate a single customer document (SET format)."""
    first = random.choice(FIRST_NAMES)
    last = random.choice(LAST_NAMES)
    city, country = random.choice(CITIES)
    tier = random.choices(TIERS, weights=TIER_WEIGHTS)[0]

    # Random subset of tags
    num_tags = random.randint(0, 4)
    tags = random.sample(TAGS, num_tags)
    tags_str = ", ".join(f'"{t}"' for t in tags)

    email = f"{first.lower()}.{last.lower()}{customer_id}@example.com"
    # Use Decimal for monetary values (balance) - mix of dec suffix and regular floats
    balance = round(random.uniform(0, 5000), 2)
    balance_str = f"{balance}dec" if random.random() > 0.3 else str(balance)  # 70% decimal
    registered = random_date(2020, 2024)

    return (
        f'name = "{first} {last}", '
        f'email = "{email}", '
        f'city = "{city}", '
        f'country = "{country}", '
        f'tier = "{tier}", '
        f"balance = {balance_str}, "
        f"registered_at = {registered}, "
        f"tags = [{tags_str}]"
    )


def generate_product(product_id):
    """Generate a single product document with varied descriptions."""
    category = random.choice(list(CATEGORIES.keys()))
    subcategory = random.choice(CATEGORIES[category])
    brand = random.choice(BRANDS)

    # Generate product name with more variety
    adjectives = [
        "Premium",
        "Classic",
        "Pro",
        "Ultra",
        "Eco",
        "Smart",
        "Mini",
        "Max",
        "Elite",
        "Basic",
        "Deluxe",
        "Essential",
        "Advanced",
        "Lite",
        "Plus",
        "Supreme",
        "Original",
        "Select",
        "Prime",
        "Signature",
        "Compact",
        "Professional",
        "Everyday",
        "Ultimate",
        "Core",
    ]
    name_styles = [
        f"{random.choice(adjectives)} {subcategory} {random_string(4).upper()}",
        f"{brand} {random.choice(adjectives)} {subcategory}",
        f"{subcategory} {random.choice(adjectives)} Edition",
        f"{random.choice(adjectives)} {brand} {subcategory}",
        f"The {brand} {subcategory} {random.choice(adjectives)}",
    ]
    name = random.choice(name_styles)

    # Generate more varied descriptions for FTS
    base_desc = random.choice(PRODUCT_DESCRIPTIONS[category])

    # Additional description variations
    value_props = [
        f"The {brand} {name} delivers exceptional quality.",
        f"Experience the difference with {brand}'s attention to detail.",
        "Designed for those who appreciate quality craftsmanship.",
        f"A top choice among {subcategory.lower()} enthusiasts.",
        "Trusted by thousands of satisfied customers worldwide.",
        "Combines innovation with proven reliability.",
        f"Sets the standard for {subcategory.lower()} excellence.",
        "The go-to choice for discerning buyers.",
    ]

    audience_props = [
        f"Perfect for {subcategory.lower()} enthusiasts.",
        "Ideal for both beginners and experts alike.",
        "Great for everyday use or special occasions.",
        "Suitable for home, office, or travel.",
        f"A must-have for anyone serious about {subcategory.lower()}.",
        "Makes an excellent gift for any occasion.",
        "Popular among professionals and hobbyists.",
        "Loved by families everywhere.",
    ]

    description = f"{base_desc} {random.choice(value_props)} {random.choice(audience_props)}"

    # Price and cost (cost is 30-70% of price)
    # Use Decimal for monetary values - mix of dec suffix and regular floats
    price = round(random.uniform(10, 2000), 2)
    cost = round(price * random.uniform(0.3, 0.7), 2)
    price_str = f"{price}dec" if random.random() > 0.3 else str(price)  # 70% decimal
    cost_str = f"{cost}dec" if random.random() > 0.3 else str(cost)  # 70% decimal

    stock = random.randint(0, 500)
    rating = round(random.uniform(1, 5), 1)
    reviews_count = random.randint(0, 500)
    is_active = random.random() > 0.1  # 90% active

    # Specs as nested object
    weight = round(random.uniform(0.1, 50), 2)
    width = random.randint(5, 100)
    height = random.randint(5, 100)
    depth = random.randint(5, 50)

    # Generate embedding from name + description
    embedding_text = f"{name} {description}"
    embedding = generate_embedding(embedding_text)
    # Format with fixed-point notation to avoid scientific notation (e.g., 8.7e-05)
    embedding_str = ", ".join(f"{v:.6f}" for v in embedding)

    return (
        f'name = "{escape_string(name)}", '
        f'description = "{escape_string(description)}", '
        f'category = "{category}", '
        f'subcategory = "{subcategory}", '
        f'brand = "{brand}", '
        f"price = {price_str}, "
        f"cost = {cost_str}, "
        f"stock = {stock}, "
        f"rating = {rating}, "
        f"reviews_count = {reviews_count}, "
        f"is_active = {str(is_active).lower()}, "
        f"specs = {{weight_kg: {weight}, dimensions: {{width_cm: {width}, height_cm: {height}, depth_cm: {depth}}}}}, "
        f"embedding = [{embedding_str}]"
    )


def generate_order(order_id, num_customer, num_product):
    """Generate a single order document with embedded items."""
    customer_id = random.randint(1, num_customer)
    status = random.choices(ORDER_STATUSES, weights=STATUS_WEIGHTS)[0]

    # Generate 1-5 items
    num_items = random.randint(1, 5)
    items = []
    subtotal = 0

    for _ in range(num_items):
        product_id = random.randint(1, num_product)
        quantity = random.randint(1, 5)
        unit_price = round(random.uniform(10, 500), 2)
        # Use Decimal for monetary values - mix of dec suffix and regular floats
        unit_price_str = f"{unit_price}dec" if random.random() > 0.3 else str(unit_price)  # 70% decimal
        items.append(f"{{product: product:{product_id}, quantity: {quantity}, unit_price: {unit_price_str}}}")
        subtotal += quantity * unit_price

    items_str = ", ".join(items)

    discount_percent = random.choice([0, 0, 0, 5, 10, 15, 20, 25])  # Most orders have no discount
    shipping_cost = round(random.uniform(0, 25), 2) if subtotal < 100 else 0
    total = round(subtotal * (1 - discount_percent / 100) + shipping_cost, 2)

    # Use Decimal for monetary totals - mix of dec suffix and regular floats
    subtotal_str = f"{round(subtotal, 2)}dec" if random.random() > 0.3 else str(round(subtotal, 2))  # 70% decimal
    shipping_cost_str = f"{shipping_cost}dec" if random.random() > 0.3 else str(shipping_cost)  # 70% decimal
    total_str = f"{total}dec" if random.random() > 0.3 else str(total)  # 70% decimal

    created = random_datetime(2022, 2024)

    return (
        f"customer = customer:{customer_id}, "
        f'status = "{status}", '
        f"subtotal = {subtotal_str}, "
        f"discount_percent = {discount_percent}, "
        f"shipping_cost = {shipping_cost_str}, "
        f"total = {total_str}, "
        f"created_at = {created}, "
        f"items = [{items_str}]"
    )


def generate_review(review_id, num_customer, num_product):
    """Generate a single review document with varied text."""
    customer_id = random.randint(1, num_customer)
    product_id = random.randint(1, num_product)
    rating = random.choices([1, 2, 3, 4, 5], weights=[5, 10, 15, 35, 35])[0]

    if rating >= 4:
        title = random.choice(REVIEW_TITLES_POSITIVE)
    elif rating <= 2:
        title = random.choice(REVIEW_TITLES_NEGATIVE)
    else:
        title = random.choice(REVIEW_TITLES_NEUTRAL)

    # Generate varied review text by combining different components
    # Number of sentences varies (2-5) for more natural variety
    num_sentences = random.randint(2, 5)

    if rating >= 4:
        # Positive review - pick from different categories
        pools = [
            REVIEW_INTRO_POSITIVE,
            REVIEW_QUALITY_POSITIVE,
            REVIEW_VALUE_POSITIVE,
            REVIEW_RECOMMEND_POSITIVE,
            REVIEW_CLOSING_POSITIVE,
        ]
    elif rating <= 2:
        # Negative review
        pools = [
            REVIEW_INTRO_NEGATIVE,
            REVIEW_QUALITY_NEGATIVE,
            REVIEW_VALUE_NEGATIVE,
            REVIEW_WARNING_NEGATIVE,
            REVIEW_CLOSING_NEGATIVE,
        ]
    else:
        # Neutral review
        pools = [
            REVIEW_INTRO_NEUTRAL,
            REVIEW_QUALITY_NEUTRAL,
            REVIEW_VALUE_NEUTRAL,
            REVIEW_CLOSING_NEUTRAL,
        ]

    # Select random pools and random sentence from each
    selected_pools = random.sample(pools, min(num_sentences, len(pools)))
    text_parts = [random.choice(pool) for pool in selected_pools]
    text = " ".join(text_parts)

    helpful_count = random.randint(0, 50)
    created = random_datetime(2022, 2024)

    return (
        f"customer = customer:{customer_id}, "
        f"product = product:{product_id}, "
        f"rating = {rating}, "
        f'title = "{escape_string(title)}", '
        f'text = "{escape_string(text)}", '
        f"helpful_count = {helpful_count}, "
        f"created_at = {created}"
    )


# ============================================================
# Batch insert functions
# ============================================================


def create_batch(collection, batch_id, docs_with_ids):
    """Create a batch of documents with explicit IDs."""
    # Each item is (id, set_assignments) where set_assignments is "field = value, ..."
    statements = [f"CREATE {collection}:{doc_id} SET {set_assignments}" for doc_id, set_assignments in docs_with_ids]
    query = "; ".join(statements)
    success, error = exec_query(query, timeout=60)
    if not success:
        print(f"\nBatch {batch_id} for {collection} failed: {error}")
        return 0
    return len(docs_with_ids)


def load_collection(name, total, generator, generator_args=()):
    """Load a collection with batched creates (explicit IDs)."""
    print(f"\nLoading {name}: {total:,} documents...")

    inserted = 0
    start_time = time.time()

    # Generate all batches with explicit IDs
    batches = []
    batch = []
    for i in range(1, total + 1):
        doc_content = generator(i, *generator_args)
        batch.append((i, doc_content))  # (id, content)
        if len(batch) >= BATCH_SIZE:
            batches.append((len(batches), batch))
            batch = []
    if batch:
        batches.append((len(batches), batch))

    total_batches = len(batches)

    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(create_batch, name, b[0], b[1]): b for b in batches}

        completed = 0
        for future in as_completed(futures):
            result = future.result()
            inserted += result
            completed += 1

            if completed % 50 == 0 or completed == total_batches:
                elapsed = time.time() - start_time
                rate = inserted / elapsed if elapsed > 0 else 0
                pct = completed / total_batches * 100
                print(f"\r  [{pct:5.1f}%] {inserted:>10,} docs | {rate:,.0f} docs/sec", end="", flush=True)

    elapsed = time.time() - start_time
    rate = inserted / elapsed if elapsed > 0 else 0
    print(f"\r  [100.0%] {inserted:>10,} docs | {rate:,.0f} docs/sec | {elapsed:.1f}s")

    return inserted


# ============================================================
# Edge creation
# ============================================================

# Edge volumes
NUM_FOLLOWS = 50_000  # Social graph: customer follows customer
NUM_WISHLISTED = 30_000  # Wishlist: customer wishlisted product
NUM_VIEWED = 100_000  # Views: customer viewed product
NUM_SIMILAR = 15_000  # Recommendations: product similar_to product
NUM_BOUGHT_TOGETHER = 20_000  # Frequently bought together

EDGE_BATCH_SIZE = 50


def generate_edge_batch(edge_type, batch_data):
    """Generate a batch of RELATE statements."""
    statements = []
    for from_id, to_id, data in batch_data:
        if data:
            data_str = ", ".join(f"{k} = {v}" for k, v in data.items())
            statements.append(f"RELATE {from_id}->{edge_type}->{to_id} SET {data_str} RETURN NONE")
        else:
            statements.append(f"RELATE {from_id}->{edge_type}->{to_id} RETURN NONE")
    return statements


def insert_edge_batch(batch_id, statements):
    """Insert a batch of edges."""
    # Execute as single batch query
    query = "; ".join(statements)
    success, error = exec_query(query, timeout=60)
    if not success:
        print(f"\nEdge batch {batch_id} failed: {error}")
        return 0
    return len(statements)


def create_edges():
    """Create graph edges between entities using RELATE."""
    print("\nCreating graph edges...")

    total_edges = 0
    overall_start = time.time()

    # 1. FOLLOWS edges (social graph)
    print(f"\n  Creating follows edges ({NUM_FOLLOWS:,})...")
    start_time = time.time()

    batches = []
    batch = []
    edges_planned = 0
    used_pairs = set()

    while edges_planned < NUM_FOLLOWS:
        customer_id = random.randint(1, NUM_CUSTOMER)
        target_id = random.randint(1, NUM_CUSTOMER)
        if customer_id != target_id and (customer_id, target_id) not in used_pairs:
            used_pairs.add((customer_id, target_id))
            # Add edge data
            since_year = random.randint(2020, 2024)
            mutual = random.random() > 0.7  # 30% mutual follows
            batch.append(
                (
                    f"customer:{customer_id}",
                    f"customer:{target_id}",
                    {"since": since_year, "mutual": str(mutual).lower()},
                )
            )
            edges_planned += 1

            if len(batch) >= EDGE_BATCH_SIZE:
                batches.append((len(batches), generate_edge_batch("follows", batch)))
                batch = []

    if batch:
        batches.append((len(batches), generate_edge_batch("follows", batch)))

    edges_created = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(insert_edge_batch, b[0], b[1]): b for b in batches}
        for future in as_completed(futures):
            edges_created += future.result()

    elapsed = time.time() - start_time
    print(f"    Created {edges_created:,} follows edges in {elapsed:.1f}s")
    total_edges += edges_created

    # 2. WISHLISTED edges (customer -> product)
    print(f"\n  Creating wishlisted edges ({NUM_WISHLISTED:,})...")
    start_time = time.time()

    batches = []
    batch = []
    edges_planned = 0
    used_pairs = set()

    while edges_planned < NUM_WISHLISTED:
        customer_id = random.randint(1, NUM_CUSTOMER)
        product_id = random.randint(1, NUM_PRODUCT)
        if (customer_id, product_id) not in used_pairs:
            used_pairs.add((customer_id, product_id))
            added_at = random_date(2022, 2024)
            priority = random.choice(["low", "medium", "high"])
            batch.append(
                (
                    f"customer:{customer_id}",
                    f"product:{product_id}",
                    {"added_at": added_at, "priority": f'"{priority}"'},
                )
            )
            edges_planned += 1

            if len(batch) >= EDGE_BATCH_SIZE:
                batches.append((len(batches), generate_edge_batch("wishlisted", batch)))
                batch = []

    if batch:
        batches.append((len(batches), generate_edge_batch("wishlisted", batch)))

    edges_created = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(insert_edge_batch, b[0], b[1]): b for b in batches}
        for future in as_completed(futures):
            edges_created += future.result()

    elapsed = time.time() - start_time
    print(f"    Created {edges_created:,} wishlisted edges in {elapsed:.1f}s")
    total_edges += edges_created

    # 3. VIEWED edges (customer -> product) - most common interaction
    print(f"\n  Creating viewed edges ({NUM_VIEWED:,})...")
    start_time = time.time()

    batches = []
    batch = []
    edges_planned = 0

    # Allow multiple views of same product (no dedup)
    while edges_planned < NUM_VIEWED:
        customer_id = random.randint(1, NUM_CUSTOMER)
        product_id = random.randint(1, NUM_PRODUCT)
        view_count = random.randint(1, 10)
        last_viewed = random_datetime(2023, 2024)
        batch.append(
            (f"customer:{customer_id}", f"product:{product_id}", {"view_count": view_count, "last_viewed": last_viewed})
        )
        edges_planned += 1

        if len(batch) >= EDGE_BATCH_SIZE:
            batches.append((len(batches), generate_edge_batch("viewed", batch)))
            batch = []

    if batch:
        batches.append((len(batches), generate_edge_batch("viewed", batch)))

    edges_created = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(insert_edge_batch, b[0], b[1]): b for b in batches}
        for future in as_completed(futures):
            edges_created += future.result()

    elapsed = time.time() - start_time
    print(f"    Created {edges_created:,} viewed edges in {elapsed:.1f}s")
    total_edges += edges_created

    # 4. SIMILAR_TO edges (product -> product) - recommendation graph
    print(f"\n  Creating similar_to edges ({NUM_SIMILAR:,})...")
    start_time = time.time()

    batches = []
    batch = []
    edges_planned = 0
    used_pairs = set()

    while edges_planned < NUM_SIMILAR:
        product_id = random.randint(1, NUM_PRODUCT)
        target_id = random.randint(1, NUM_PRODUCT)
        if product_id != target_id and (product_id, target_id) not in used_pairs:
            used_pairs.add((product_id, target_id))
            similarity = round(random.uniform(0.5, 0.99), 2)
            reason = random.choice(["category", "brand", "price_range", "co_purchased", "description"])
            batch.append(
                (f"product:{product_id}", f"product:{target_id}", {"similarity": similarity, "reason": f'"{reason}"'})
            )
            edges_planned += 1

            if len(batch) >= EDGE_BATCH_SIZE:
                batches.append((len(batches), generate_edge_batch("similar_to", batch)))
                batch = []

    if batch:
        batches.append((len(batches), generate_edge_batch("similar_to", batch)))

    edges_created = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(insert_edge_batch, b[0], b[1]): b for b in batches}
        for future in as_completed(futures):
            edges_created += future.result()

    elapsed = time.time() - start_time
    print(f"    Created {edges_created:,} similar_to edges in {elapsed:.1f}s")
    total_edges += edges_created

    # 5. BOUGHT_TOGETHER edges (product -> product)
    print(f"\n  Creating bought_together edges ({NUM_BOUGHT_TOGETHER:,})...")
    start_time = time.time()

    batches = []
    batch = []
    edges_planned = 0
    used_pairs = set()

    while edges_planned < NUM_BOUGHT_TOGETHER:
        product_id = random.randint(1, NUM_PRODUCT)
        target_id = random.randint(1, NUM_PRODUCT)
        if product_id != target_id and (product_id, target_id) not in used_pairs:
            used_pairs.add((product_id, target_id))
            co_purchase_count = random.randint(5, 500)
            confidence = round(random.uniform(0.1, 0.9), 2)
            batch.append(
                (
                    f"product:{product_id}",
                    f"product:{target_id}",
                    {"count": co_purchase_count, "confidence": confidence},
                )
            )
            edges_planned += 1

            if len(batch) >= EDGE_BATCH_SIZE:
                batches.append((len(batches), generate_edge_batch("bought_together", batch)))
                batch = []

    if batch:
        batches.append((len(batches), generate_edge_batch("bought_together", batch)))

    edges_created = 0
    with ThreadPoolExecutor(max_workers=WORKERS) as executor:
        futures = {executor.submit(insert_edge_batch, b[0], b[1]): b for b in batches}
        for future in as_completed(futures):
            edges_created += future.result()

    elapsed = time.time() - start_time
    print(f"    Created {edges_created:,} bought_together edges in {elapsed:.1f}s")
    total_edges += edges_created

    overall_elapsed = time.time() - overall_start
    print(f"\n  Total edges created: {total_edges:,} in {overall_elapsed:.1f}s")

    return total_edges


# ============================================================
# Main
# ============================================================


def main():
    print("=" * 60)
    print("StellarDB E-commerce Test Data Loader")
    print("=" * 60)
    print()
    print("Target volumes:")
    print(f"  - customer: {NUM_CUSTOMER:,}")
    print(f"  - product:  {NUM_PRODUCT:,}")
    print(f"  - order:    {NUM_ORDER:,}")
    print(f"  - review:   {NUM_REVIEW:,}")
    print(f"  - Total:    {NUM_CUSTOMER + NUM_PRODUCT + NUM_ORDER + NUM_REVIEW:,} documents")
    print()
    print("Graph edges:")
    print(f"  - follows:         {NUM_FOLLOWS:,} (customer -> customer)")
    print(f"  - wishlisted:      {NUM_WISHLISTED:,} (customer -> product)")
    print(f"  - viewed:          {NUM_VIEWED:,} (customer -> product)")
    print(f"  - similar_to:      {NUM_SIMILAR:,} (product -> product)")
    print(f"  - bought_together: {NUM_BOUGHT_TOGETHER:,} (product -> product)")
    total_edges = NUM_FOLLOWS + NUM_WISHLISTED + NUM_VIEWED + NUM_SIMILAR + NUM_BOUGHT_TOGETHER
    print(f"  - Total:          {total_edges:,} edges")
    print()
    print(f"Settings: batch_size={BATCH_SIZE}, workers={WORKERS}")
    print()

    overall_start = time.time()
    total_inserted = 0

    # Create database
    print(f"Creating database '{DATABASE}'...")
    try:
        r = requests.post(f"{BASE_URL}/databases", json={"name": DATABASE}, timeout=10)
        if r.status_code == 200:
            print(f"  - {DATABASE}: OK")
        else:
            # Database may already exist, continue anyway
            print(f"  - {DATABASE}: {r.text[:100]} (continuing)")
    except Exception as e:
        print(f"  - Failed to create database: {e}")
        return

    # Create collections with schemas
    print("\nCreating collections...")

    schemas = [
        (
            "customer",
            """
            DEFINE COLLECTION customer (
                name string REQUIRED,
                email string REQUIRED,
                city string REQUIRED,
                country string REQUIRED,
                tier string REQUIRED,
                balance decimal,
                registered_at datetime,
                tags [string]
            )
        """,
        ),
        (
            "product",
            """
            DEFINE COLLECTION product (
                name string REQUIRED,
                description string,
                category string REQUIRED,
                subcategory string REQUIRED,
                brand string REQUIRED,
                price decimal REQUIRED,
                cost decimal REQUIRED,
                stock int,
                rating float,
                reviews_count int,
                is_active bool,
                specs {SCHEMA FLEXIBLE, weight_kg float},
                embedding [float]
            )
        """,
        ),
        (
            "order",
            """
            DEFINE COLLECTION order (
                customer ref REQUIRED,
                status string REQUIRED,
                subtotal decimal REQUIRED,
                discount_percent int,
                shipping_cost decimal,
                total decimal REQUIRED,
                created_at datetime,
                items [{product ref, quantity int, unit_price decimal}] REQUIRED
            )
        """,
        ),
        (
            "review",
            """
            DEFINE COLLECTION review (
                customer ref REQUIRED,
                product ref REQUIRED,
                rating int REQUIRED,
                title string,
                text string,
                helpful_count int,
                created_at datetime
            )
        """,
        ),
    ]

    for name, schema in schemas:
        success, error = exec_query(schema)
        if success:
            print(f"  - {name}: OK")
        else:
            print(f"  - {name}: FAILED - {error}")
            return

    # Create indexes
    print("\nCreating indexes...")

    indexes = [
        ("customer", "email"),
        ("customer", "city"),
        ("customer", "tier"),
        ("customer", "country"),
        ("product", "category"),
        ("product", "brand"),
        ("product", "price"),
        ("product", "rating"),
        ("order", "customer"),
        ("order", "status"),
        ("order", "created_at"),
        ("order", "total"),
        ("review", "customer"),
        ("review", "product"),
        ("review", "rating"),
    ]

    for collection, field in indexes:
        success, error = exec_query(f"CREATE INDEX ON {collection}({field})")
        if success:
            print(f"  - {collection}({field}): OK")
        else:
            print(f"  - {collection}({field}): FAILED - {error}")

    # Create FTS (full-text search) indexes
    print("\nCreating FTS indexes...")

    fts_indexes = [
        ("product", "name, description"),  # Search products by name and description
        ("review", "title, text"),  # Search reviews by title and content
    ]

    for collection, fields in fts_indexes:
        success, error = exec_query(f"CREATE INDEX ON {collection}({fields}) FULLTEXT")
        if success:
            print(f"  - {collection}({fields}) FULLTEXT: OK")
        else:
            print(f"  - {collection}({fields}) FULLTEXT: FAILED - {error}")

    # Create HNSW vector indexes
    hnsw_indexes = [
        ("product", "embedding", 384, "COSINE"),
    ]

    print("\nCreating HNSW vector indexes...")
    for collection, field, dim, dist in hnsw_indexes:
        success, error = exec_query(f"CREATE INDEX ON {collection}({field}) HNSW DIMENSION {dim} DIST {dist}")
        if success:
            print(f"  - {collection}({field}) HNSW DIMENSION {dim}: OK")
        else:
            print(f"  - {collection}({field}): FAILED - {error}")

    # Load data
    total_inserted += load_collection("customer", NUM_CUSTOMER, generate_customer)
    total_inserted += load_collection("product", NUM_PRODUCT, generate_product)
    total_inserted += load_collection("order", NUM_ORDER, generate_order, (NUM_CUSTOMER, NUM_PRODUCT))
    total_inserted += load_collection("review", NUM_REVIEW, generate_review, (NUM_CUSTOMER, NUM_PRODUCT))

    # Create graph edges
    total_edges = create_edges()

    # Summary
    overall_elapsed = time.time() - overall_start
    overall_rate = total_inserted / overall_elapsed if overall_elapsed > 0 else 0

    print()
    print("=" * 60)
    print("DONE!")
    print("=" * 60)
    print(f"  Total documents: {total_inserted:,}")
    print(f"  Total edges:     {total_edges:,}")
    print(f"  Total time:      {overall_elapsed:.1f} seconds")
    print(f"  Overall rate:    {overall_rate:,.0f} docs/sec")
    print()
    print("// Sample queries to try:")
    print()
    print("// " + "=" * 50)
    print("// BASIC QUERIES")
    print("// " + "=" * 50)
    print()
    print("  // Top customers by balance")
    print("  SELECT name, balance, tier FROM customer ORDER balance DESC LIMIT 10;")
    print()
    print("  // Products with profit margin")
    print("  SELECT name, price, cost, price - cost AS margin FROM product ORDER price - cost DESC LIMIT 10;")
    print()
    print("  // Orders by effective total (with discount)")
    print("  SELECT * FROM order ORDER subtotal * (1 - discount_percent / 100) DESC LIMIT 10;")
    print()
    print("  // Average rating by category")
    print("  SELECT category, AVG(rating), COUNT(*) FROM product GROUP category;")
    print()
    print("  // Reviews with customer info (subquery)")
    print("  SELECT rating, title, (SELECT name FROM $parent.customer)[0] AS reviewer FROM review LIMIT 10;")
    print()
    print("// " + "=" * 50)
    print("// DATETIME, DURATION & RANGE (NEW!)")
    print("// " + "=" * 50)
    print()
    print("  // Native datetime type with time:: functions")
    print("  SELECT time::now() AS current_time;")
    print()
    print("  // Extract year/month from datetime fields")
    print("  SELECT name, registered_at, time::year(registered_at) AS year, time::month(registered_at) AS month")
    print("  FROM customer LIMIT 10;")
    print()
    print("  // Filter orders by year")
    print("  SELECT * FROM order WHERE time::year(created_at) = 2024 LIMIT 10;")
    print()
    print("  // Orders from specific month")
    print("  SELECT total, created_at FROM order")
    print("  WHERE time::year(created_at) = 2023 AND time::month(created_at) = 6")
    print("  ORDER created_at DESC LIMIT 10;")
    print()
    print("  // Duration literals and functions")
    print("  SELECT duration::secs(1h30m) AS seconds;")
    print("  SELECT duration::millis(500ms) AS millis;")
    print()
    print("  // Range iteration in FROM clause")
    print("  SELECT * FROM 1..10;")
    print("  SELECT * FROM 1..100 WHERE math::modulo($value, 2) = 0;  // even numbers")
    print("  SELECT $value * $value AS square FROM 1..10;")
    print()
    print("// " + "=" * 50)
    print("// MIXED NUMERIC TYPES (Int, Float, Decimal)")
    print("// " + "=" * 50)
    print()
    print("  // Data uses mixed types: 70% Decimal (with 'dec' suffix), 30% Float/Int")
    print("  // Cross-type comparisons work seamlessly!")
    print()
    print("  // Range query - works with any numeric type")
    print("  SELECT name, price FROM product WHERE price >= 100 AND price <= 500 ORDER price LIMIT 10;")
    print()
    print("  // Cross-type arithmetic - Decimal and Float mix")
    print("  SELECT name, price, cost, price - cost AS margin FROM product ORDER margin DESC LIMIT 10;")
    print()
    print("  // Filter by exact value - 100, 100.0, and 100dec all compare equal")
    print("  SELECT * FROM customer WHERE balance = 100;")
    print()
    print("  // Order by Decimal field with Int comparison in WHERE")
    print("  SELECT name, total FROM order WHERE total > 1000 ORDER total DESC LIMIT 10;")
    print()
    print("  // Aggregation across mixed types")
    print("  SELECT category, AVG(price), SUM(price), MIN(price), MAX(price) FROM product GROUP category;")
    print()
    print("// " + "=" * 50)
    print("// REFERENCE DEREFERENCING")
    print("// " + "=" * 50)
    print()
    print("  // Automatic reference dereferencing - access fields on referenced documents")
    print("  // The 'customer' field is a reference (customer:123), .name automatically loads the document")
    print()
    print("  // Orders with customer name and tier (no subquery needed!)")
    print("  SELECT customer.name, customer.tier, total FROM order LIMIT 10;")
    print()
    print("  // Reviews with product info")
    print("  SELECT rating, title, product.name AS product_name, product.category FROM review LIMIT 10;")
    print()
    print("  // Chained dereferencing - orders with customer's city")
    print("  SELECT total, customer.name, customer.city, customer.country FROM order WHERE total > 500 LIMIT 10;")
    print()
    print("  // Compare with old subquery approach (both work, but dereferencing is simpler)")
    print("  // Old: SELECT total, (SELECT name FROM $parent.customer)[0] AS cust_name FROM order")
    print("  // New: SELECT total, customer.name AS cust_name FROM order")
    print()
    print("// " + "=" * 50)
    print("// FULL-TEXT SEARCH")
    print("// " + "=" * 50)
    print()
    print("  // FTS: Search products by name/description")
    print(
        '  SELECT name, description, fts::score() AS relevance FROM product WHERE description @@ "premium quality" ORDER relevance DESC LIMIT 10;'
    )
    print()
    print("  // FTS: Search reviews with score ordering")
    print(
        '  SELECT title, text, rating, fts::score() AS score FROM review WHERE text @@ "excellent quality" ORDER score DESC LIMIT 10;'
    )
    print()
    print("  // FTS: Multi-field search with boosts (title weighted 2x)")
    print(
        '  SELECT title, text, fts::score() AS score FROM review WHERE MATCH(title^2, text) @@ "recommend" ORDER score DESC LIMIT 10;'
    )
    print()
    print("// " + "=" * 50)
    print("// VECTOR SEARCH")
    print("// " + "=" * 50)
    print()
    print("  // Vector search (find similar products by embedding)")
    print(
        "  SELECT name, vector::distance() AS dist FROM product WHERE embedding <|5|> (SELECT VALUE embedding FROM product:1)[0];"
    )
    print()
    print("  // Hybrid search (combine FTS and vector)")
    print("  LET sample_vec = (SELECT VALUE embedding FROM product:1)[0];")
    print("  SELECT * FROM search::rrf([")
    print("    (SELECT id, name, vector::distance() AS d FROM product WHERE embedding <|5|> $sample_vec),")
    print('    (SELECT id, name, fts::score() AS s FROM product WHERE name @@ "premium")')
    print("  ], 10);")
    print()
    print("// " + "=" * 50)
    print("// GRAPH QUERIES")
    print("// " + "=" * 50)
    print()
    print("  // Who does customer:1 follow?")
    print("  SELECT ->follows->customer.* AS following FROM customer:1;")
    print()
    print("  // Who follows customer:1?")
    print("  SELECT <-follows<-customer.* AS followers FROM customer:1;")
    print()
    print("  // What products did customer:1 wishlist?")
    print("  SELECT ->wishlisted->product.* AS wishlist FROM customer:1;")
    print()
    print("  // Products similar to product:1")
    print("  SELECT ->similar_to->product.* AS similar FROM product:1;")
    print()
    print("  // Frequently bought together with product:1")
    print("  SELECT ->bought_together->product.* AS also_bought FROM product:1;")
    print()
    print("  // Products viewed by customer:1")
    print("  SELECT ->viewed->product.* AS viewed FROM customer:1;")
    print()
    print("  // Get edge data (follows relationship details)")
    print("  SELECT ->follows AS follow_edges FROM customer:1;")
    print()
    print("  // Create a new follow relationship")
    print("  RELATE customer:1->follows->customer:2 SET since = 2024, mutual = false;")
    print()
    print("  // Delete a follow relationship")
    print("  DELETE customer:1->follows->customer:2;")
    print()
    print("// " + "=" * 50)
    print("// MULTI-FIELD SELECTION (NEW!)")
    print("// " + "=" * 50)
    print()
    print("  // Select specific edge fields with custom names")
    print("  // Syntax: ->edge.{field1, alias: field2, ...}")
    print()
    print("  // Get follows edge data with selected fields")
    print("  SELECT ->follows.{since, mutual, target: to} FROM customer:1;")
    print()
    print("  // Wishlist with aliased fields")
    print("  SELECT ->wishlisted.{added: added_at, priority, product: to} FROM customer:1;")
    print()
    print("  // View data with specific fields")
    print("  SELECT ->viewed.{views: view_count, last: last_viewed, product: to} FROM customer:1;")
    print()
    print("  // Similar products with similarity score")
    print("  SELECT ->similar_to.{score: similarity, reason, product: to} FROM product:1;")
    print()
    print("  // Bought together with confidence")
    print("  SELECT ->bought_together.{times: count, confidence, product: to} FROM product:1;")
    print()
    print("  // Select specific node fields after traversal")
    print("  SELECT ->follows->customer.{name, email, tier} FROM customer:1;")
    print()
    print("  // Combine: edge fields + reference dereferencing")
    print("  // 'to' is a reference, .name dereferences it automatically")
    print("  SELECT ->wishlisted.{priority, product_name: to.name, product_price: to.price} FROM customer:1;")
    print()
    print("  // Get follower names directly (no need for .* then filter)")
    print("  SELECT ->follows.{since, follower_name: to.name, follower_tier: to.tier} FROM customer:1;")
    print()
    print("// " + "=" * 50)
    print("// COMPLEX QUERIES (using multiple features)")
    print("// " + "=" * 50)
    print()
    print("  // Find products wishlisted by people I follow")
    print("  // (social recommendation)")
    print("  SELECT ->follows->customer->wishlisted->product.* AS recommended")
    print("  FROM customer:1;")
    print()
    print("  // Same query with multi-field selection - get specific product fields")
    print("  SELECT ->follows->customer->wishlisted->product.{name, price, category} AS recommended")
    print("  FROM customer:1;")
    print()
    print("  // Products viewed by followers of customer:1")
    print("  SELECT <-follows<-customer->viewed->product.* AS trending_in_network")
    print("  FROM customer:1;")
    print()
    print("  // Get wishlist with product names using reference dereferencing")
    print("  SELECT ->wishlisted.{priority, product_name: to.name, added: added_at} FROM customer:1;")
    print()
    print("  // Social graph analysis: who follows whom with names")
    print("  SELECT ->follows.{since, mutual, friend: to.name, friend_tier: to.tier} FROM customer:1;")
    print()
    print("// " + "=" * 50)
    print("// CONTROL FLOW (IF / FOR / BREAK / CONTINUE)")
    print("// " + "=" * 50)
    print()
    print("  // --- LET + IF: classify customer tier based on spending ---")
    print("  LET total_spent = (SELECT VALUE SUM(total) FROM order WHERE customer = customer:1)[0].`sum(total)`;")
    print('  IF $total_spent > 5000 THEN "whale"')
    print('  ELSE IF $total_spent > 1000 THEN "regular"')
    print('  ELSE "newcomer" END;')
    print()
    print("  // --- IF as expression inside LET ---")
    print("  LET c = (SELECT * FROM customer:1)[0];")
    print("  LET discount = IF $c.tier = \"platinum\" THEN 25")
    print("    ELSE IF $c.tier = \"gold\" THEN 15")
    print("    ELSE IF $c.tier = \"silver\" THEN 10")
    print("    ELSE 0 END;")
    print("  $discount;")
    print()
    print("  // --- FOR: compute per-category stats in one batch ---")
    print('  LET cats = ["Electronics", "Clothing", "Home", "Sports"];')
    print("  FOR $cat IN $cats DO")
    print("    (SELECT $cat AS category, COUNT(*) AS total, AVG(price) AS avg_price")
    print("     FROM product WHERE category = $cat)[0]")
    print("  END;")
    print()
    print("  // --- FOR over query results: enrich each order with customer name ---")
    print("  LET recent = (SELECT * FROM order ORDER created_at DESC LIMIT 5);")
    print("  FOR $ord IN $recent DO")
    print("    LET cust = (SELECT name, tier FROM $ord.customer)[0];")
    print("    {order_id: $ord.id, total: $ord.total, customer: $cust.name, tier: $cust.tier}")
    print("  END;")
    print()
    print("  // --- FOR + IF: flag orders needing attention ---")
    print("  LET orders = (SELECT * FROM order WHERE status = \"pending\" LIMIT 20);")
    print("  FOR $o IN $orders DO")
    print('    IF $o.total > 500 THEN {id: $o.id, alert: "high-value pending"}')
    print('    ELSE IF $o.discount_percent > 20 THEN {id: $o.id, alert: "big discount"}')
    print("    ELSE CONTINUE END")
    print("  END;")
    print()
    print("  // --- FOR + BREAK: find first platinum customer with balance > 1000 ---")
    print("  LET platinums = (SELECT * FROM customer WHERE tier = \"platinum\" LIMIT 100);")
    print("  FOR $c IN $platinums DO")
    print("    IF $c.balance > 1000 THEN")
    print("      LET found = {name: $c.name, balance: $c.balance};")
    print("      $found;")
    print("      BREAK")
    print("    END")
    print("  END;")
    print()
    print("  // --- Nested FOR: cross-product recommendations ---")
    print('  LET brands = ["TechPro", "EcoLine"];')
    print('  LET categories = ["Electronics", "Home"];')
    print("  FOR $b IN $brands DO")
    print("    FOR $c IN $categories DO")
    print("      (SELECT name, price FROM product")
    print("       WHERE brand = $b AND category = $c")
    print("       ORDER rating DESC LIMIT 1)[0]")
    print("    END")
    print("  END;")
    print()
    print("  // --- FOR with range: batch price analysis ---")
    print("  FOR $bucket IN [0, 100, 500, 1000] DO")
    print("    LET upper = $bucket + 100;")
    print("    (SELECT $bucket AS price_from, COUNT(*) AS products")
    print("     FROM product WHERE price >= $bucket AND price < $upper)[0]")
    print("  END;")
    print()
    print("// " + "=" * 50)
    print("// ULTIMATE QUERY: All features combined")
    print("// " + "=" * 50)
    print("""
  // This query demonstrates ALL StellarDB features:
  // - Document queries with filtering and sorting
  // - Aggregations (COUNT, AVG)
  // - Subqueries (correlated)
  // - Full-text search with scoring
  // - Vector similarity search
  // - Graph traversal (social + product recommendations)
  // - Multi-field selection: ->edge.{field1, alias: field2}
  // - Reference dereferencing: customer.name (auto-loads referenced doc)
  // - Computed fields and expressions
  //
  // Scenario: Find premium products that:
  // 1. Match a text search for "quality"
  // 2. Are similar to a reference product (vector search)
  // 3. Were wishlisted by users I follow (graph)
  // 4. Have good reviews
  //
  // Note: Run parts separately if full query is too complex

  // Part 1: FTS + Aggregation - Find well-reviewed "quality" products
  SELECT
    id,
    name,
    category,
    price,
    fts::score() AS text_relevance,
    (SELECT AVG(rating) AS avg FROM review WHERE product = $parent.id)[0].avg AS avg_review,
    (SELECT COUNT(*) AS cnt FROM review WHERE product = $parent.id)[0].cnt AS review_count
  FROM product
  WHERE description @@ "quality" AND is_active = true AND price > 100
  ORDER text_relevance DESC
  LIMIT 20;

  // Part 2: Vector search - Find products similar to product:1
  SELECT
    id,
    name,
    category,
    vector::distance() AS similarity
  FROM product
  WHERE embedding <|10|> (SELECT VALUE embedding FROM product:1)[0];

  // Part 3: Graph - Products wishlisted by my followers (with multi-field selection)
  SELECT ->follows->customer->wishlisted.{
    priority,
    product_name: to.name,
    product_price: to.price,
    product_category: to.category
  } AS social_recommendations
  FROM customer:1;

  // Part 4: Graph - Products frequently bought with my viewed products
  SELECT ->viewed->product->bought_together.{
    times: count,
    confidence,
    product_name: to.name
  } AS cross_sell
  FROM customer:1;

  // Part 5: Reviews with automatic reference dereferencing (no subquery needed!)
  // Old approach: (SELECT name, tier FROM $parent.customer)[0] AS reviewer
  // New approach: customer.name, customer.tier - automatic dereferencing!
  SELECT
    rating,
    title,
    text,
    customer.name AS reviewer_name,
    customer.tier AS reviewer_tier,
    product.name AS product_name,
    product.category AS product_category
  FROM review
  WHERE product = product:1 AND rating >= 4
  ORDER helpful_count DESC
  LIMIT 10;

  // Part 6: Follow relationships with full context
  SELECT ->follows.{
    since,
    mutual,
    friend_name: to.name,
    friend_city: to.city,
    friend_tier: to.tier
  } AS my_network
  FROM customer:1;
""")
    print()


if __name__ == "__main__":
    main()
